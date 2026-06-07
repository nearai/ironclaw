# ironclaw_agent_loop tests

Own framework-level tests for loop state, strategies, families, and executor
behavior.

## Test surfaces

- Use the public executor entry point and real built-in families for behavior
  tests.
- Use `ironclaw_agent_loop::test_support` for host fixtures and scripted model
  or capability outcomes.
- Keep private strategy-slot access inside crate unit tests when needed; do not
  make production APIs public for integration-test convenience.

## Adding tests

- Add executor tests when behavior depends on interaction between strategies,
  checkpoints, input draining, cancellation, or host calls.
- Add state tests when checkpoint payload shape, ring behavior, or slot
  validation changes.
- Add family tests when registry identity or default composition changes.

## Common mistakes

- Do not test only helper functions when the bug is in executor behavior.
- Do not duplicate `ironclaw_reborn` driver-adapter coverage here.
- Do not assert on raw prompt/model/tool payloads; framework state should carry
  refs and summaries only.
