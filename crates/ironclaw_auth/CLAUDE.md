# ironclaw_auth Guardrails

- Own product-facing auth vocabulary and fake services only.
- Keep Reborn auth code independent from V1 route handlers, V1 pending state, V1 extension manager authority, and V1 secret-store implementation details.
- Serializable records may contain hashes, ids, handles, statuses, and redacted metadata. They must not contain raw OAuth state, PKCE verifiers, authorization codes, tokens, secret values, provider response bodies, backend internals, or host paths.
- Raw OAuth callback material may appear only in non-serializable one-shot inputs to provider exchange boundaries.
- Manual token values must move through `SecretString` inputs and must not appear in `Debug`, errors, projections, or docs. Tests may use sentinel values only to prove redaction.
- Use strong newtypes for auth-domain identifiers and hashes; deserialize through validation.
- Public wire enums must use stable snake_case serde names.
- Fakes should fail closed and model important state transitions closely enough that production consumers cannot depend on unsafe shortcuts.
