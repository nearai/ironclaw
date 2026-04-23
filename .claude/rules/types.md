---
paths:
  - "src/**"
  - "crates/**"
  - "tests/**"
---
# Typed Internals — No Stringly-Typed Values Inside the System

**Every domain value gets a specialized type.** Raw `String` is a boundary
format — accepted from user input, JSON/HTTP payloads, the database,
and untrusted external APIs — and converted to a domain type at the
earliest opportunity. Everything flowing between internal modules must
carry a type that makes misuse a compile error.

- **Identifiers** → newtypes (`CredentialName`, `ExtensionName`,
  `ThreadId`, `UserId`). Never `String`, `&str`, or `uuid::Uuid` alone.
- **Fixed small sets** → enums with `#[serde(rename_all = "snake_case")]`
  or explicit `#[serde(rename = "...")]`. Never compare strings like
  `status == "in_progress"`.
- **Units, shapes, modes** → enums (`SandboxPolicy`, `ExecutionMode`,
  `ThreadState`). Never booleans-plus-magic-strings.

Two values with the same shape but different meanings must be
different types. The compiler is the only durable enforcement —
comments, naming, and code review are not.

## Why

Identity confusion has shipped four times in recent history:

| PR | Surface | What went wrong |
|----|---------|-----------------|
| #2561 | settings restart | `owner_id` round-tripped through a string, lost type on reload |
| #2473 | Slack relay OAuth | nonce stored under wrong scope — wrong `user_id` vs gateway owner id |
| #2512 | Slack relay OAuth | state lookup compared strings across two callers that had diverged |
| #2574 | auth-gate display | inline fallback re-derived extension name, returned `telegram_bot_token` where `telegram` was expected |

Same shape every time: a string-typed value passes through more than
one layer, one layer treats it as one meaning, another as a different
meaning, and the compiler has nothing to say. Newtypes would have
turned each into a compile error.

## Extension/Auth identity invariant

See `CLAUDE.md` → "Extension/Auth Invariants" for routing rules. The
types live in `crates/ironclaw_common/src/identity.rs`:

- [`CredentialName`] — backend secret identity (e.g.
  `telegram_bot_token`, `google_oauth_token`). Used for secrets-store
  keys, gate resume payloads, credential injection.
- [`ExtensionName`] — user-facing installed extension/channel identity
  (e.g. `telegram`, `gmail`). Used for onboarding UI, setup/configure
  routing, Python action dispatch. Hyphens fold to underscores at
  construction time.

Never cast between them. Never recompute one from the other by string
manipulation — resolve through
`AuthManager::resolve_extension_name_for_auth_flow`.

## Canonical newtype template

New newtypes use this single shape. Validation happens on the wire
(`try_from`) and at explicit construction (`::new`), both routed
through a shared `validate(&str)`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct MyId(String);

impl MyId {
    fn validate(s: &str) -> Result<(), MyIdError> { /* ... */ }

    pub fn new(raw: impl Into<String>) -> Result<Self, MyIdError> {
        let s = raw.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str { &self.0 }
    pub fn into_inner(self) -> String { self.0 }
}

impl TryFrom<String> for MyId {
    type Error = MyIdError;
    fn try_from(value: String) -> Result<Self, MyIdError> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for MyId {
    fn as_ref(&self) -> &str { &self.0 }
}

impl fmt::Display for MyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<MyId> for String {
    fn from(id: MyId) -> Self { id.0 }
}
// Deliberately no `From<String>` / `From<&str>` — infallible
// conversion would silently bypass validation.
// Deliberately no `Deref<Target = str>` — auto-deref would let
// `&id` silently coerce to `&str`, which is the implicit-conversion
// pattern this rule exists to prevent.
```

Rules baked into the template:

- `#[serde(try_from = "String")]` — wire validation matches
  construction; do not use `#[serde(transparent)]` on a newly added
  validated newtype.
- Shared `validate(&str)` — one source of truth for the invariant.
- `impl Into<String>` on `new` — avoids a clone for owned-`String`
  callers; still accepts `&str`.
- Explicit `as_str()` / `as_ref()` / `into_inner()` — every boundary
  crossing is visible in the source.
- Match-on-string-literals means the type should be an enum. Fix the
  type.
- Don't return `String` from an internal function — return the newtype.
- Don't compare a newtype against a format-string-built `String`.
  `format!("{}_token", extension_name) == credential_name.as_str()`
  rebuilds the bug #2574 fixed — route through the shared resolver.

## Legacy exception — `#[serde(transparent)]` identity types

`CredentialName` and `ExtensionName` predate this template. They use
`#[serde(transparent)]` + derived `Deserialize` and deliberately do
*not* revalidate on the wire — the `serde_does_not_revalidate` test
in `identity.rs` locks that contract in, because legacy persisted rows
may not satisfy the current rule.

Don't "migrate" them to `try_from` — you will break rehydration of
pre-existing DB rows. New code must still construct them through
`::new()`. The `from_trusted(String)` helper on those two types is a
legacy escape hatch for values handed over from a typed upstream (DB
row, parsed `ExtensionManifest` field); do not copy that pattern onto
new newtypes.

Review flag: `#[serde(transparent)]` on a newly added validated
newtype, or a `from_trusted` helper on anything other than the two
legacy identity types. References: PR #2685, PR #2681, PR #2687.

## Byte-length vs. character-length

A validator using `s.len()` measures bytes. If the error message says
"N characters", switch to `s.chars().count()`. Pick one and match the
message.

## Wire-stable enums

Enums serialized over the network or persisted to the DB are part of
the public contract.

Derive `Serialize` + `Deserialize` with
`#[serde(rename_all = "snake_case")]`. Add enum helper methods for
wire/UI rendering — never `format!("{:?}", ...)`. `format!("{:?}",
status)` emits `"InProgress"` while snake_case serde emits
`"in_progress"`; the drift has already shipped (#2669 `mission_list`
vs `mission_complete`).

**Migrations from `String` must preserve every historical value.**
When replacing a stringly-typed wire field with an enum, add
`#[serde(alias = "...")]` for every value any running producer still
emits. Grep the tree; check staging/production logs. Add a round-trip
deserialization test with raw legacy JSON. Reference: PR #2678
`JobResultStatus` rejected `"error"` / `"stuck"` / case variants on
rollout.

## Wire-contract field naming

A boolean or enum exposed to the web UI has exactly one canonical
snake_case name on the wire (`engine_v2_enabled`) and one canonical
JS accessor (`window.bootstrap.engineV2Enabled`). Reading the same
value from ad-hoc `data.engine_v2` inside a surface file is a bug —
it will diverge. Delete duplicate fields in response structs (PR
#2665 shipped both `engine_v2` and `engine_v2_enabled` in one struct).
Frontend reads the flag from bootstrap globals, not from response
bodies. References: PR #2683, PR #2702.

## Applies to

`src/**`, `crates/**`, `tests/**`. Any code inside the IronClaw
workspace. The rule doesn't apply to wire payloads (which are `String`
by virtue of JSON), log lines, or error messages — those *are* the
boundary.
