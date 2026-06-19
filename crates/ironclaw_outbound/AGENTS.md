# Agent Map — ironclaw_outbound

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for dependencies and backend feature shape.
- Use `docs/reborn/contracts/events-projections.md` as the source of truth for outbound egress/subscription policy.

## What This Crate Owns

- Metadata-only outbound egress policy state.
- Per-thread notification policy for explicit fanout and progress opt-in.
- Projection subscription cursor checkpoints scoped to actor, thread, and `ProjectionScope`.
- Outbound delivery attempt/status metadata for retry and support-visible workflows.
- Policy seams for projection access authorization and reply-target revalidation.

## Do Not Move In Here

- Transport sends or concrete Slack/Telegram/Web payload validation.
- Canonical transcript, thread, turn, or projection content mutation.
- Raw prompts, message bodies, tool inputs/outputs, secrets, host paths, backend error details, or unredacted payload snippets.
- Product adapter rendering/delivery logic; adapters consume candidates and report delivery status.

## Validation

- Fast local check: `cargo test -p ironclaw_outbound`
- Backend parity check without live Postgres: `IRONCLAW_SKIP_POSTGRES_TESTS=1 cargo test -p ironclaw_outbound --all-features`
- Lint check: `cargo clippy -p ironclaw_outbound --all-targets --all-features -- -D warnings`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`

## Agent Notes

- Keep external push targets as candidates until `ReplyTargetBindingValidator` revalidates the route.
- Authorization-revoked pushes must record sanitized delivery failure and return no sendable target.
- Delivery failure records must not mark turns/runs failed or mutate canonical transcript/projection state.
- Prefer service-level tests when policy gates subscription, delivery, persistence, or authorization side effects.
