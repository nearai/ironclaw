---
paths:
  - "src/**"
  - "crates/**"
  - "tests/**"
---
# Typed Internals — No Stringly-Typed Values Inside the System

**Internal values must have types that reflect what they mean.** Raw `String`
is the boundary type — accepted from user input, JSON/HTTP payloads, the
database, and untrusted external APIs — and it should be converted to a
domain type as soon as possible. Everything that moves between internal
modules should carry a type that makes misuse a compile error.

Concretely:

- **Identifiers** → newtypes (`CredentialName`, `ExtensionName`, `ThreadId`,
  `UserId`). Never `String`, `&str`, or `uuid::Uuid` alone.
- **Fixed small sets** → enums with `#[serde(rename_all = "snake_case")]`
  or explicit `#[serde(rename = "...")]`. Never compare strings like
  `status == "in_progress"`.
- **Units, shapes, modes** → enums (`SandboxPolicy`, `ExecutionMode`,
  `ThreadState`). Never booleans-plus-magic-strings.

If two values have the same shape (`String`, `u64`, whatever) but different
meanings, they must be different types. The compiler is the only durable
enforcement of "don't mix these up" — comments, naming, and code review
are not.

## Why

Identity confusion has shipped four times in recent history:

| PR | Surface | What went wrong |
|----|---------|-----------------|
| #2561 | settings restart | `owner_id` round-tripped through a string, lost type on reload |
| #2473 | Slack relay OAuth | nonce stored under wrong scope — wrong `user_id` vs gateway owner id |
| #2512 | Slack relay OAuth | state lookup compared strings across two callers that had diverged |
| #2574 | auth-gate display | inline fallback re-derived extension name, returned `telegram_bot_token` where `telegram` was expected |

All four bugs are the same shape: a string-typed value passes through more
than one layer, one layer treats it as one meaning, another treats it as a
different meaning, and the compiler has nothing to say. Newtypes would
have made each of these a type error.

## The Extension/Auth identity invariant

See `CLAUDE.md` → "Extension/Auth Invariants" for the routing rules. The
types live in `crates/ironclaw_common/src/identity.rs`:

- [`CredentialName`] — backend secret identity (e.g. `telegram_bot_token`,
  `google_oauth_token`). Used for secrets-store keys, gate resume
  payloads, credential injection.
- [`ExtensionName`] — user-facing installed extension/channel identity
  (e.g. `telegram`, `gmail`). Used for onboarding UI, setup/configure
  routing, Python action dispatch. Hyphens fold to underscores at
  construction time because extensions are invoked as Python attribute
  accesses.

Never cast between them. Never recompute one from the other by string
manipulation — resolve through `AuthManager::resolve_extension_name_for_auth_flow`.

## When to add a newtype

Add one when **all** of the following are true:

1. The value is a *name*, *id*, or *key* — something whose shape is
   incidental to its meaning.
2. It flows through **more than one module** or crosses a **type
   boundary** (struct field, function parameter, return type).
3. Mixing it up with another same-shape value would be a **silent
   runtime bug**, not a compile error.

If all three hold, make it a newtype. Put it in
`crates/ironclaw_common/src/identity.rs` (or a module-local spot if its
blast radius is genuinely one crate).

### Newtype template

Use `#[serde(transparent)]` so on-wire and on-disk representation stays a
plain string — legacy persisted rows must keep deserializing cleanly.
Validate at explicit construction sites (`new`, `try_from`, `from_str`),
not on the wire. Provide a `from_trusted(String)` escape hatch for values
sourced from a typed upstream (DB row, registry entry) where the caller
already trusts the shape.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MyId(String);

impl MyId {
    pub fn new(raw: impl Into<String>) -> Result<Self, MyIdError> { ... }
    pub fn from_trusted(raw: String) -> Self { Self(raw) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl AsRef<str> for MyId { ... }        // explicit via `.as_ref()`
impl TryFrom<String> for MyId { ... }   // validating
impl From<MyId> for String { ... }      // infallible
// Deliberately no `From<String>` or `From<&str>` — infallible
// conversion would silently bypass validation.
// Deliberately no `Deref<Target = str>` — auto-deref would let
// `&my_id` silently coerce to `&str`, which is the implicit-conversion
// pattern this whole module exists to prevent. Use `.as_str()` /
// `.as_ref()` at the call site so the boundary is visible.
```

## Don'ts

- **Don't add `From<&str>` or `From<String>` for an identity newtype.**
  That relaxes the invariant. If a caller has a raw string, they should
  have to choose `new` (validate) or `from_trusted` (documented opt-out)
  — the choice itself is the audit trail.
- **Don't compare a newtype against a format-string-built `String`.**
  If you find yourself writing `format!("{}_token", extension_name) ==
  credential_name.as_str()`, you've rebuilt the bug #2574 fixed. Route
  through the shared resolver instead.
- **Don't use `#[serde(try_from = "String")]` for identity newtypes
  without a migration plan.** Existing persisted rows may not satisfy the
  current rule; `transparent` + explicit validation at construction
  preserves them while still gaining type distinctness.
- **Don't match on string literals for a value that should be an enum.**
  `match status.as_str() { "ready" => ... }` means status should be an
  enum. Fix the type.
- **Don't downgrade a typed value back to `String` except at a system
  boundary.** A `String` returned from an internal function is a
  regression — return the type.

## Using `from_trusted` safely

`from_trusted(String)` is an escape hatch for internal-to-internal
handoff — not a shortcut for external input.

The distinction is **where the string came from at this call site**,
not what format it lives in on disk:

- **Trusted** = the value has already passed through a typed parse step
  that validated it. A DB row loaded through a typed repo is trusted
  (the repo's column type is the newtype, or a previous `::new()` call
  populated it). A skill-manifest registry entry *as a typed field on a
  parsed `ExtensionManifest` struct* is trusted. A test fixture with a
  literal string is trusted.
- **Untrusted** = the value is still a raw `String` / `&str` pulled
  from a payload that was not itself validated as this newtype. A JSON
  field you just deserialized into a `String`, a CLI arg, an HTTP body,
  a webhook frame, a config file, a registry JSON field read as
  `Value::String` — all untrusted. Use `::new(..)` and propagate the
  error.

The literal text "registry entry" can be either: a field on a parsed
manifest struct is trusted; reaching into the raw JSON for the same
name is not.

Review flag: added `from_trusted` in `src/channels/**`, `src/bridge/**`,
handler files, or `*_handler.rs`. References: PR #2685
`IncomingMessage::with_thread`, PR #2681 MCP client constructors,
PR #2687 `extensions_install_handler`.

## Validated newtype template

`new(impl Into<String>) -> Result<Self, _>` and `TryFrom<String>` must
share a private `validate(&str)` helper. Both validate on a borrow,
then consume the owned `String` — never re-allocate. `impl Into<String>`
on `new` is what lets `&str` *and* owned `String` callers avoid a clone.

```rust
impl MyId {
    fn validate(s: &str) -> Result<(), MyIdError> { /* ... */ }

    pub fn new(raw: impl Into<String>) -> Result<Self, MyIdError> {
        let s = raw.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }
}

impl TryFrom<String> for MyId {
    type Error = MyIdError;
    fn try_from(value: String) -> Result<Self, MyIdError> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}
```

### Validated newtypes must gate `Deserialize` (new types only)

For a **new** validated newtype whose wire contract must match its
construction invariant, never combine `#[derive(Deserialize)]` with
`#[serde(transparent)]` — `transparent` bypasses `::new()` entirely. Use
`#[serde(try_from = "String")]` and implement `TryFrom<String>` as
above.

**Exception — existing identity newtypes**: `CredentialName` and
`ExtensionName` in `crates/ironclaw_common/src/identity.rs`
intentionally use `#[serde(transparent)]` + derived `Deserialize` and
do *not* re-validate on the wire. This is deliberate: legacy persisted
rows may not satisfy the current rule, and the `serde_does_not_revalidate`
test locks that contract in. Don't "fix" those to `try_from` — you will
break rehydration of pre-existing DB rows. Validation for those types
happens at explicit `::new()` construction sites.

Review flag: `#[serde(transparent)]` on a *newly added* type whose
`::new()` returns `Result`, without a documented legacy-persistence
reason.

### Byte-length vs. character-length

A validator using `s.len()` measures bytes. If the error message says
"N characters", switch to `s.chars().count()`. Pick one and match the
message.

## Wire-stable enums

Enums serialized over the network or persisted to the DB are part of
the public contract.

**Never use `Debug` as a serializer.** `format!("{:?}", status)` emits
`"InProgress"` while snake_case serde emits `"in_progress"` — this
drift has already shipped (#2669 `mission_list` vs `mission_complete`).
Derive `Serialize`/`Deserialize` with
`#[serde(rename_all = "snake_case")]` and add enum helper methods — not
`format!("{:?}", ...)` — for any wire or UI rendering.

**Migrations from `String` must preserve every historical value.** When
replacing a stringly-typed wire field with an enum, add
`#[serde(alias = "...")]` for every value any running producer still
emits. Grep the tree; check staging/production logs. Add a round-trip
deserialization test with raw legacy JSON. Reference: PR #2678
`JobResultStatus` rejected `"error"` / `"stuck"` / case variants on
rollout.

## Wire-contract field naming

A boolean or enum exposed to the web UI has exactly one canonical
snake_case name on the wire (`engine_v2_enabled`) and one canonical JS
accessor (`window.bootstrap.engineV2Enabled`). Reading the same value
from ad-hoc `data.engine_v2` inside a surface file is a bug — it will
diverge. Delete duplicate fields in response structs (PR #2665 shipped
both `engine_v2` and `engine_v2_enabled` in one struct). Frontend reads
the flag from bootstrap globals, not from response bodies. References:
PR #2683, PR #2702.

## Applies to

`src/**`, `crates/**`, `tests/**`. Any code inside the IronClaw
workspace. The rule doesn't apply to wire payloads (which are `String`
by virtue of JSON), log lines, or error messages — those *are* the
boundary.
