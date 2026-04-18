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
    pub fn new(raw: impl AsRef<str>) -> Result<Self, MyIdError> { ... }
    pub fn from_trusted(raw: String) -> Self { Self(raw) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl TryFrom<String> for MyId { ... }   // validating
impl From<MyId> for String { ... }      // infallible
// Deliberately no `From<String>` — that would silently bypass validation.
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

## Applies to

`src/**`, `crates/**`, `tests/**`. Any code inside the IronClaw
workspace. The rule doesn't apply to wire payloads (which are `String`
by virtue of JSON), log lines, or error messages — those *are* the
boundary.
