# ironclaw_auth Guardrails

- Own product-facing auth vocabulary and fake services only.
- Exception: `ProviderBackedCredentialAccountService` may live here because refresh serialization and status projection belong at the `CredentialAccountService` boundary, while raw provider/token material stays behind `AuthProviderClient` and secret boundaries.
- Keep Reborn auth code independent from V1 route handlers, V1 pending state, V1 extension manager authority, and V1 secret-store implementation details.
- Serializable records may contain hashes, ids, handles, statuses, and redacted metadata. They must not contain raw OAuth state, PKCE verifiers, authorization codes, tokens, secret values, provider response bodies, backend internals, or host paths.
- Raw OAuth callback material may appear only in non-serializable one-shot inputs to provider exchange boundaries.
- Token refresh must go through `CredentialAccountService::refresh_account` and `AuthProviderClient::refresh_token`. Refresh requests/results stay behind host-mediated auth/provider boundaries, revalidate scope/provider/ownership/grants, and project recoverable failures as stable statuses rather than raw provider detail.
- `AuthFlowRecordSource` is the auth-owned read/list seam for product interaction read models. Composition crates may wire it, but should not define parallel auth-flow snapshot traits.
- Cleanup lifecycle handling must be ownership-aware and idempotent. Deactivate/uninstall may revoke extension-owned accounts or remove grants, but reusable/shared/system credentials must not be deleted by default; partial failures should surface stable quarantine categories only.
- Manual token values must move through `SecretString` inputs and must not appear in `Debug`, errors, projections, or docs. Tests may use sentinel values only to prove redaction.
- Credential recovery/account-selection projections must expose only stable status/reason categories and redacted authorized choices. Revalidate scope, provider, configured status, ownership, and grants when selecting a `CredentialAccountId`; ids are not authority by themselves.
- Use strong newtypes for auth-domain identifiers and hashes; deserialize through validation.
- Public wire enums must use stable snake_case serde names.
- Fakes should fail closed and model important state transitions closely enough that production consumers cannot depend on unsafe shortcuts.
