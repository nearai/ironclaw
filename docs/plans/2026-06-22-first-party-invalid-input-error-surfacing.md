# Plan: structured first-party invalid-input error surfacing

## Problem

`builtin.trigger_create` can reject model input for two different reasons:

- serde shape errors, such as the old flat `{ "cron": "...", "timezone": "UTC" }`
  shape or a missing `schedule.kind`.
- trigger semantic validation errors, such as an invalid timezone, invalid cron
  cadence, or one-shot local time that falls into a DST gap.

Today both collapse to `RuntimeDispatchErrorKind::InputEncode` with no
model-visible field detail. The agent loop then produces a generic observation:

```json
{
  "status": "error",
  "summary": "Capability failed with invalid_input.",
  "detail": { "kind": "generic_failure", "failure_kind": "invalid_input" }
}
```

That preserves the broad classification but loses the fact the model needs to
self-correct, for example `schedule.kind` is missing.

## Goal

Preserve the existing coarse error taxonomy, and add a narrow structured detail
payload for first-party runtime validation failures:

```text
RuntimeFailureKind::InvalidInput
  -> CapabilityFailureKind::InvalidInput
    -> CapabilityFailureDetail::InvalidInput {
         issues: [{ path: "schedule.kind", code: MissingRequired }]
       }
```

The model should receive the same structured invalid-input observation already
used for provider JSON-schema validation:

```json
{
  "status": "error",
  "summary": "Tool input failed schema validation.",
  "detail": {
    "kind": "invalid_input",
    "issues": [
      {
        "path": "schedule.kind",
        "code": "missing_required",
        "expected": "cron or once"
      }
    ]
  }
}
```

## Non-goals

- Do not add a sixth coarse failure taxonomy.
- Do not encode field-level validation cases into `RuntimeFailureKind` or
  `CapabilityFailureKind`.
- Do not serialize structured issue JSON into `safe_summary`.
- Do not make `ironclaw_host_api` depend on `ironclaw_turns`.
- Do not redesign all first-party capabilities in this PR. `trigger_create` is
  the first producer.

## Design decisions

### 1. Name the payload as detail, not kind

Use names that make the type's role obvious:

```rust
pub enum DispatchFailureDetail {
    InvalidInput { issues: Vec<DispatchInputIssue> },
}

pub struct DispatchInputIssue {
    pub path: String,
    pub code: DispatchInputIssueCode,
    pub expected: Option<String>,
    pub received: Option<String>,
    pub schema_path: Option<String>,
}

pub enum DispatchInputIssueCode {
    MissingRequired,
    UnexpectedField,
    TypeMismatch,
    InvalidValue,
}
```

`DispatchFailureDetail` is not a replacement for `RuntimeDispatchErrorKind`,
`RuntimeFailureKind`, or `CapabilityFailureKind`. It is supplemental sanitized
repair detail.

### 2. Put the neutral carrier at the dispatch boundary

The first neutral boundary crossed by first-party failures is
`ironclaw_host_api::DispatchError::FirstParty`, so attach the detail there:

```rust
pub enum DispatchError {
    FirstParty {
        kind: RuntimeDispatchErrorKind,
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
    },
    // unchanged variants...
}
```

Reasoning:

- first-party handlers can produce the detail without depending on the agent
  loop or turn crates;
- `CapabilityInvocationError` already preserves dispatch kind and safe summary,
  so preserving detail there is natural;
- upper runtime conversion can carry the same detail into `RuntimeCapabilityFailure`;
- the loop boundary can convert neutral issue types to
  `CapabilityFailureDetail::InvalidInput` exactly once.

### 3. Keep `safe_summary` as fallback text

`safe_summary` stays a short, sanitized fallback for UI/logging/model replay
when no structured observation is available.

Recommended trigger input summary:

```text
trigger_create input failed validation
```

Do not include raw cron expressions, prompts, or serialized issue arrays in
`safe_summary`.

### 4. Do not hand-parse the whole trigger input unless required

The code-judo move is to avoid replacing serde with a parallel parser. Keep
serde as the authoritative typed parser for the happy path, and add a small
diagnostic classifier for common malformed shapes only when serde fails.

Pseudocode:

```rust
async fn create_trigger(input: Value, ...) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerCreateInput = TriggerCreateInput::deserialize(&input)
        .map_err(|serde_error| trigger_create_shape_error(&input, serde_error))?;

    let schedule_kind = input.schedule.kind();
    let schedule = input.schedule.into_schedule()
        .map_err(|error| trigger_schedule_error(schedule_kind, error))?;

    let next_run_at = next_run_at_for_schedule(&schedule, now)
        .map_err(|error| trigger_next_run_error(schedule_kind, error))?;

    // unchanged persistence path
}
```

`trigger_create_shape_error` should inspect only stable JSON shape facts:

```rust
fn trigger_create_shape_error(raw: &Value, _: serde_json::Error) -> FirstPartyCapabilityError {
    let issues = classify_trigger_create_shape(raw);
    invalid_trigger_input(issues)
}

fn classify_trigger_create_shape(raw: &Value) -> Vec<DispatchInputIssue> {
    let Some(root) = raw.as_object() else {
        return vec![type_mismatch("input", "object")];
    };

    let mut issues = Vec::new();
    required_string(root, "name", "name", &mut issues);
    required_string(root, "prompt", "prompt", &mut issues);

    unexpected_fields(root, &["name", "prompt", "schedule"], "", &mut issues);

    let Some(schedule) = root.get("schedule") else {
        issues.push(missing_required("schedule").expected("object with kind"));
        return issues;
    };
    let Some(schedule) = schedule.as_object() else {
        issues.push(type_mismatch("schedule", "object"));
        return issues;
    };

    match schedule.get("kind") {
        None | Some(Value::Null) => {
            issues.push(missing_required("schedule.kind").expected("cron or once"))
        }
        Some(Value::String(kind)) if kind == "cron" => {
            unexpected_fields(schedule, &["kind", "expression", "timezone"], "schedule.", &mut issues);
            required_string(schedule, "expression", "schedule.expression", &mut issues);
            required_string(schedule, "timezone", "schedule.timezone", &mut issues);
        }
        Some(Value::String(kind)) if kind == "once" => {
            unexpected_fields(schedule, &["kind", "at", "timezone"], "schedule.", &mut issues);
            required_string(schedule, "at", "schedule.at", &mut issues);
            required_string(schedule, "timezone", "schedule.timezone", &mut issues);
        }
        Some(Value::String(_)) => {
            issues.push(invalid_value("schedule.kind").expected("cron or once"))
        }
        Some(_) => issues.push(type_mismatch("schedule.kind", "string")),
    }

    issues
}
```

This classifier is intentionally not a second complete deserializer. Serde still
owns the typed parse. The classifier exists only to turn common shape failures
into repair hints. `TriggerCreateInput` and `TriggerScheduleInput` use
`deny_unknown_fields`, so the serde contract remains aligned with the published
`additionalProperties: false` schema. The classifier is a known duplicate-truth
tradeoff and should be replaced by a single decoder/validator path in a follow-up
if this grows.

### 5. Use schedule branch context for semantic validation

For semantic schedule errors, do not classify by matching English reason text.
`ironclaw_triggers` exposes typed validation kinds while keeping the human
`reason` for display/logging.

Pseudocode:

```rust
pub enum TriggerScheduleValidationKind {
    InvalidTimezone,
    InvalidDateTime,
    AmbiguousDateTime,
    NonexistentDateTime,
    EmptyCronExpression,
    InvalidCronFieldCount,
    InvalidCronExpression,
    SecondLevelCadence,
    NoUpcomingFireTime,
    SubMinuteCadence,
    NoFutureFireTime,
}

pub enum TriggerRecordValidationKind {
    NameEmpty,
    NameTooLong,
    PromptEmpty,
    PromptTooLong,
    Other,
}

enum TriggerScheduleInput {
    Cron { expression: String, timezone: String },
    Once { at: String, timezone: String },
}

impl TriggerScheduleInput {
    fn kind(&self) -> TriggerScheduleInputKind { ... }
}

fn trigger_schedule_error(
    kind: TriggerScheduleInputKind,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    let issue = match error {
        TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::InvalidTimezone,
            ..
        } => {
            invalid_value("schedule.timezone").expected("valid IANA timezone name")
        }
        TriggerError::InvalidSchedule { .. } => {
            match kind {
                TriggerScheduleInputKind::Cron => invalid_value("schedule.expression")
                    .expected("five-, six-, or seven-field cron with at least one-minute cadence"),
                TriggerScheduleInputKind::Once => invalid_value("schedule.at")
                    .expected("YYYY-MM-DDTHH:MM:SS valid in the selected timezone"),
            }
        }
        _ => invalid_value("schedule").expected("valid trigger schedule"),
    };

    invalid_trigger_input(vec![issue])
}
```

Record validation maps the typed `NameEmpty`, `NameTooLong`, `PromptEmpty`, and
`PromptTooLong` kinds to `name` / `prompt` issues. Persistence and worker
invariants use `Other` and stay generic because they are not model-repairable
`trigger_create` input hints.

## Propagation pseudocode

### `ironclaw_host_api`

```rust
pub enum DispatchError {
    FirstParty {
        kind: RuntimeDispatchErrorKind,
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
    },
}
```

### `ironclaw_host_runtime::FirstPartyCapabilityError`

```rust
pub enum FirstPartyCapabilityError {
    Dispatch {
        kind: RuntimeDispatchErrorKind,
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
        usage: Option<ResourceUsage>,
    },
}

impl FirstPartyCapabilityError {
    pub fn invalid_input_issues(
        safe_summary: impl Into<String>,
        issues: Vec<DispatchInputIssue>,
    ) -> Self {
        Self::Dispatch {
            kind: RuntimeDispatchErrorKind::InputEncode,
            safe_summary: Some(safe_summary.into()),
            detail: Some(DispatchFailureDetail::InvalidInput { issues }),
            usage: None,
        }
    }
}
```

### `FirstPartyRuntimeAdapter`

```rust
match error {
    FirstPartyCapabilityError::Dispatch {
        kind,
        safe_summary,
        detail,
        ..
    } => Err(DispatchError::FirstParty {
        kind,
        safe_summary,
        detail,
    }),
}
```

### `ironclaw_capabilities::CapabilityInvocationError`

```rust
pub enum CapabilityInvocationError {
    Dispatch {
        kind: DispatchFailureKind,
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
    },
}

impl From<DispatchError> for CapabilityInvocationError {
    DispatchError::FirstParty { kind, safe_summary, detail } => Self::Dispatch {
        kind: DispatchFailureKind::Runtime(kind),
        safe_summary,
        detail,
    }
}
```

All non-first-party dispatch variants set `detail: None`.

### `ironclaw_host_runtime::RuntimeCapabilityFailure`

```rust
pub struct RuntimeCapabilityFailure {
    pub capability_id: CapabilityId,
    pub kind: RuntimeFailureKind,
    pub message: Option<String>,
    pub detail: Option<DispatchFailureDetail>,
}

impl RuntimeCapabilityFailure {
    pub fn new(...) -> Self {
        Self { detail: None, ... }
    }

    pub fn with_detail(mut self, detail: DispatchFailureDetail) -> Self {
        self.detail = Some(detail);
        self
    }
}
```

`failure_from` copies `detail` only for `CapabilityInvocationError::Dispatch`.

### `ironclaw_loop_support`

Convert the neutral detail once:

```rust
fn runtime_model_visible_failure_to_loop(
    failure: RuntimeCapabilityFailure,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    Ok(CapabilityOutcome::Failed(CapabilityFailure {
        error_kind: model_visible_runtime_failure_kind_to_loop(failure.kind)?,
        safe_summary: runtime_failure_safe_summary(&failure, "capability invocation failed"),
        detail: runtime_failure_detail_to_loop(failure.detail),
    }))
}

fn runtime_failure_detail_to_loop(
    detail: Option<DispatchFailureDetail>,
) -> Option<CapabilityFailureDetail> {
    detail.map(|DispatchFailureDetail::InvalidInput { issues }| {
        CapabilityFailureDetail::InvalidInput {
            issues: issues.into_iter().map(dispatch_issue_to_loop).collect(),
        }
    })
}
```

`model_visible_capability_failure_observation` already renders
`CapabilityFailureDetail::InvalidInput { issues }` as structured repair output.

## Trigger issue mapping

### Shape errors

| Input problem | Path | Code | Expected |
|---|---|---|---|
| root is not object | `input` | `TypeMismatch` | `object` |
| missing `schedule` | `schedule` | `MissingRequired` | `object with kind` |
| old flat `cron` | `unexpected_field` | `UnexpectedField` | none |
| extra root field | `unexpected_field` | `UnexpectedField` | none |
| missing discriminator | `schedule.kind` | `MissingRequired` | `cron or once` |
| unknown discriminator | `schedule.kind` | `InvalidValue` | `cron or once` |
| non-string discriminator | `schedule.kind` | `TypeMismatch` | `string` |
| cron missing expression | `schedule.expression` | `MissingRequired` | `cron expression` |
| extra schedule field | `schedule.unexpected_field` | `UnexpectedField` | none |
| schedule missing timezone | `schedule.timezone` | `MissingRequired` | `IANA timezone name` |
| once missing local time | `schedule.at` | `MissingRequired` | `YYYY-MM-DDTHH:MM:SS` |

### Semantic errors

| Input problem | Path | Code | Expected |
|---|---|---|---|
| invalid timezone | `schedule.timezone` | `InvalidValue` | `valid IANA timezone name` |
| invalid cron/cadence | `schedule.expression` | `InvalidValue` | `five-, six-, or seven-field cron with at least one-minute cadence` |
| one-shot invalid local time/DST | `schedule.at` | `InvalidValue` | `YYYY-MM-DDTHH:MM:SS valid in the selected timezone` |
| no future cron slot | `schedule.expression` | `InvalidValue` | `cron expression with at least one future fire time` |
| one-shot is not in future | `schedule.at` | `InvalidValue` | `future local datetime` |

## Implementation tasks

- [ ] Replace the interrupted partial names with `DispatchFailureDetail`,
  `DispatchInputIssue`, and `DispatchInputIssueCode`.
- [ ] Thread `detail: Option<DispatchFailureDetail>` through:
  - `DispatchError::FirstParty`
  - `FirstPartyCapabilityError::Dispatch`
  - `CapabilityInvocationError::Dispatch`
  - `RuntimeCapabilityFailure`
  - `runtime_failure_to_loop`
- [ ] Keep all existing constructors defaulting to `detail: None`.
- [ ] Add a narrow first-party constructor for invalid input issues.
- [ ] Add the `trigger_create` shape classifier without replacing serde's
  canonical typed parsing.
- [ ] Add typed trigger schedule/record validation kinds and map semantic errors
  without string-matching reason text.
- [ ] Map trigger semantic validation errors using typed domain validation kinds
  plus the known schedule branch.
- [ ] Convert dispatch issues to loop `CapabilityInputIssue` at the loop
  boundary.
- [ ] Remove any safe-summary JSON serialization from the interrupted patch.

## Tests

- `crates/ironclaw_capabilities/src/error.rs`
  - `From<DispatchError>` preserves first-party detail.
- `crates/ironclaw_host_runtime/src/production.rs`
  - `failure_from` preserves dispatch detail into `RuntimeCapabilityFailure`.
- `crates/ironclaw_loop_support/src/capability_port.rs`
  - runtime invalid-input detail converts to
    `CapabilityFailureDetail::InvalidInput`.
- `crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`
  - `builtin.trigger_create` old flat shape reports `cron` unexpected and
    `schedule` missing.
  - missing `schedule.kind` reports `schedule.kind`.
  - missing `schedule.timezone` reports `schedule.timezone`.
  - invalid cron reports `schedule.expression`.
  - invalid timezone reports `schedule.timezone`.
  - type mismatches report `TypeMismatch`.
  - extra root/schedule fields report `UnexpectedField`.
- `crates/ironclaw_triggers/src/lib.rs`
  - trigger schedule/record validation errors carry typed validation kinds.
- Existing agent-loop model-observation test remains the final proof that
  `CapabilityFailureDetail::InvalidInput` becomes structured model-visible
  repair guidance.

## Verification

- `cargo fmt`
- `cargo test -p ironclaw_capabilities dispatch_error`
- `cargo test -p ironclaw_host_runtime trigger_create`
- `cargo test -p ironclaw_loop_support runtime_failure_to_loop`
- `cargo test -p ironclaw_agent_loop model_visible`
- Targeted clippy after the code patch:
  `cargo clippy -p ironclaw_host_api -p ironclaw_capabilities -p ironclaw_triggers -p ironclaw_host_runtime -p ironclaw_loop_support --all-targets -- -D warnings`

## Thermo-nuclear review loop

### Pass 1 findings

1. `RuntimeFailureDetail` is too close to `RuntimeFailureKind`; it reads like a
   competing taxonomy. Use `DispatchFailureDetail` or
   `RuntimeCapabilityFailureDetail`.
2. A full hand-written trigger input parser is spaghetti-prone and duplicates
   serde. Keep serde as the parser, and add only a diagnostic classifier for
   failed shapes.
3. JSON-in-`safe_summary` is a bad boundary leak. Structured data must travel in
   the detail field; `safe_summary` remains fallback prose.
4. String-matching every `TriggerError::InvalidSchedule` reason would be brittle.
   Use the known schedule branch for most mapping; only use a narrow timezone
   discriminator unless the trigger crate grows typed schedule sub-kinds.
5. Adding generic issue types in `ironclaw_turns` would invert dependencies.
   Keep the neutral dispatch detail in `ironclaw_host_api` and convert upward.

### Pass 1 revisions applied to this plan

- Renamed the planned neutral carrier to `DispatchFailureDetail`.
- Removed the full manual parse approach from the target design.
- Moved structured issue transport out of `safe_summary`.
- Added an explicit stop condition: if trigger semantic mapping grows, add typed
  schedule errors in `ironclaw_triggers` instead of expanding string matching.

### Pass 2 verdict

Pass 2 caught three plan defects and the plan has been revised:

1. The pseudocode moved `input.schedule` before using its branch kind. It now
   records `schedule_kind` before `into_schedule()`.
2. The first-party constructor was accidentally trigger-specific. It now accepts
   a caller-supplied safe summary.
3. Non-string `schedule.kind` was implicitly treated as missing. It now maps to
   `TypeMismatch`.

### Pass 3 verdict

The implementation review found two valid structural concerns:

1. The success path cloned the full JSON input just to support rare serde
   diagnostics. The implementation now deserializes from `&Value`.
2. Semantic validation mapping was still stringly typed through
   `TriggerError.reason`. The implementation now adds typed
   `TriggerScheduleValidationKind` and `TriggerRecordValidationKind` values in
   `ironclaw_triggers` and maps those in the host runtime.

The remaining duplicate-truth concern in `classify_trigger_create_shape` is
accepted for this PR and should be handled by a follow-up input-boundary
redesign if it grows. The current guardrail is the caller-level
`builtin_trigger_create_surfaces_structured_invalid_input_detail` matrix.

This implementation plan is acceptable if the code patch follows the constraints
above. The strongest maintainability invariant is:

```text
coarse kinds route behavior; detail payload explains repair.
```

Do not continue with an implementation that violates that invariant.
