# Error Handling Standard (AI Agents)

## Required

- Use typed errors (`thiserror`) for domain-level failures.
- Propagate errors with context instead of panicking.
- Prefer `Result<T, E>` boundaries over silent fallbacks.
- Map lower-level errors with explicit context where relevant.

## Production panic policy

- Do not introduce `.unwrap()` or `.expect()` in production paths.
- Test code may use `.unwrap()` / `.expect()` when it improves clarity.

## Examples

- Preferred: `foo().map_err(|e| MyError::Foo { reason: e.to_string() })?`
- Avoid: `foo().unwrap()` in non-test code.
