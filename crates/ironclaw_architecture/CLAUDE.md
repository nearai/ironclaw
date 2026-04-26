# ironclaw_architecture guardrails

- This crate is test-only architecture enforcement; do not add production dependencies or runtime behavior.
- Use `cargo metadata` or equivalent workspace graph checks to enforce Reborn dependency direction.
- Boundary tests should fail loudly with the exact forbidden edge and crate name.
- Keep rules conservative and explicit; update docs when intentional architecture edges change.
