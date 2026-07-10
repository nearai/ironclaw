---
paths:
  - "crates/**/*.rs"
  - "src/**/*.rs"
  - "tests/**/*.rs"
---
# Default-Backed Builder Setters

Prefer `Type::default().set_x(value).set_y(value)` over default-heavy struct
literals when a type has a meaningful `Default` and callers usually override
only a few fields.

Use this pattern for large compositional structs, config structs, request DTOs,
test fixtures, resource estimates, and similar shapes where adding a field
should not force every sparse caller to learn about it.

## Required Shape

- Setters consume and return `Self`: `pub fn set_x(mut self, x: T) -> Self`.
- For `Option<T>` fields, `set_x(value)` stores `Some(value)`.
- For string fields, accept `impl Into<String>` when the type owns a `String`.
- For collection fields, prefer iterator-based setters when callers naturally
  pass arrays, vectors, or sets.

## When To Keep A Struct Literal

Keep the literal when most fields are deliberately populated, when a test is
asserting the full serialized/persisted shape, or when the struct has required
identity/state fields and no honest default. A builder that hides required
domain data is worse than a long literal.

Do not add fluent setters just because a struct is large. Add them when they
remove repeated default boilerplate and make sparse overrides easier to review.

## Review Flags

- A new `Foo { a: ..., b: ..., ..Foo::default() }` with only one to three
  overrides on a large/default-backed type that already has
  `Foo::default().set_*`.
- A new builder API for a persistence record or runtime wiring bundle whose
  fields are all required and meaningful.
- Setter methods that take `Option<T>` for the common case; callers should not
  need to write `Some(...)` just to override a default.
