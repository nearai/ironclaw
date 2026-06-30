# Finish Backend GSuite OAuth Integration

## Summary

- Keep PR `#4100` as the installability slice: bundled Calendar/Gmail assets, lifecycle install/activate/remove, and fail-closed dispatch.
- Next work is backend-only: make Reborn auth create Google OAuth flows, exchange callbacks, store credential accounts through the existing credential setup boundary, and let GSuite missing/scope-mismatched credentials block through auth services instead of failing as a plain dispatch error.
- Do not implement WebUI v2 OAuth prompt wiring or browser E2E in this slice. File follow-up issues for those surfaces.

## Key Changes

### `ironclaw_auth`

- Add a Google provider implementation behind the existing auth ports.
- Support Google OAuth client metadata through the existing built-in/override conventions unless the implementation explicitly adds a settings-backed source and updates the relevant configuration docs in the same branch. Required fields are client id, optional client secret, redirect URI/base URL, and hosted-domain hint if already supported by existing config conventions.
- Add an OAuth start helper/service that creates `NewAuthFlow` with:
  - `AuthChallenge::OAuthUrl`
  - provider `google`
  - requested scopes
  - account label
  - PKCE/state hashes only
  - a pre-authorized `CredentialAccountUpdateBinding` whenever the flow can create or update a credential account
  - continuation of `TurnGateResume` or `SetupOnly`
- Implement Google code exchange behind `AuthProviderClient`, using host-mediated egress, storing access/refresh token material as secret handles, and returning `OAuthProviderExchange`.
- Route OAuth credential-account create/update through `CredentialSetupService`; `RebornProductAuthServices` should orchestrate flow/callback completion over the auth ports, not become the account mutation owner.
- Add refresh/status vocabulary for expired, revoked, and refresh-failed Google accounts. Wire refresh only if the current auth services already have a suitable refresh boundary; otherwise file refresh as a follow-up.

### Reborn Composition And Product Auth

- Compose the real Google provider client into `RebornProductAuthServices` instead of relying on `InMemoryAuthProductServices` for provider exchange outside tests.
- Keep Google OAuth client metadata on `RebornBuildInput` as the product/bootstrap composition seam until a settings-backed source exists.
- Add a service-level API/helper to start Google auth for required scopes and a continuation.
- Preserve callback authority:
  - callback route/caller must call `RebornProductAuthServices::handle_oauth_callback -> AuthFlowManager`
  - callback claim validates flow, scope, state, provider, and PKCE before provider exchange
  - callback completion validates the pre-authorized `CredentialAccountUpdateBinding` before account writes
  - callback dispatch failure returns a sanitized retryable error
  - retrying an already-completed callback re-dispatches the typed continuation without re-exchanging the provider code or duplicating credential writes
  - no legacy pending OAuth maps
  - no extension-manager activation side effects
  - no runtime dispatch from the callback route

### GSuite And Runtime Auth Gating

- Convert all `GoogleCredentialResolver` failures into typed recovery/setup/auth-required results before network dispatch:
  - missing account
  - unauthorized-only account set, collapsed to the same public missing-account/setup-required reason
  - missing scopes
  - expired, revoked, inactive, pending-setup, refresh-failed, or missing-access-secret account
  - ambiguous account choice
  - backend/auth/host-api failures as sanitized stable errors
- Auth-required projection must use redacted credential-account projections only. Do not expose raw account ids, secret handles, backend errors, provider bodies, host paths, state tokens, or hidden-account existence.
- Account disambiguation must use the existing account-selection/lookup APIs with requester extension identity and explicit limit/cursor pagination; do not full-scan accounts on every Gmail/Calendar invocation.
- Backend behavior should create a Reborn auth flow and emit the typed continuation needed for the existing blocked auth gate path. Keep blocked-run gate rendering/resolution ownership in the existing #3094 path; this slice must not define a second gate-resolution path.
- Keep GSuite handlers simple:
  - declare required scopes
  - resolve via `CredentialAccountService`
  - emit host-mediated HTTP requests with credential injection plans
  - fail closed

### Approval Integration

- Leave existing write-capability approval semantics intact: Calendar/Gmail writes stay `PermissionMode::Ask` with `ExternalWrite`.
- Ensure auth happens before approval when no usable Google credential exists.
- Once auth completes, the resumed turn can hit the normal approval gate for write operations.

## Test Plan

### `ironclaw_auth` Contract Tests

- Google OAuth start creates an `OAuthUrl` challenge with correct provider, scopes, state hash, PKCE hash, expiry, and continuation.
- OAuth start binds the target account create/update through a pre-authorized `CredentialAccountUpdateBinding`.
- Callback claim validates scope/state/PKCE before provider exchange.
- Callback completion rejects missing or cross-scope account-update bindings before account writes.
- Retried completed callbacks re-dispatch the typed continuation without re-exchanging the Google authorization code.
- Provider exchange stores only secret handles and never serializes raw code, verifier, access token, or refresh token.
- Google provider exchange uses host-mediated egress and returns sanitized failures without raw provider response bodies.
- Missing client configuration fails with a stable setup-required auth error.

### Reborn Composition Tests

- Service-level start -> callback -> `CredentialAccount` configured with Google provider and granted scopes.
- Provider exchange failure terminally fails the flow and emits no continuation.
- Continuation dispatch failure returns a retryable callback error and leaves enough completed-flow state to replay continuation dispatch safely.
- Continuation dispatcher resumes only `BlockedAuthGate` turns and rejects cross-scope flow/callback attempts.

### GSuite Backend Caller Tests

- Invoking Gmail/Calendar without a configured Google account blocks auth rather than returning a generic dispatch failure.
- Missing required scope starts scope-upgrade auth with the delta scopes and the correct account binding.
- Multiple configured Google accounts produces account-selection required, not an arbitrary choice, without full-scanning or exposing hidden accounts.
- Missing-access-secret, pending-setup, inactive, revoked, expired, refresh-failed, backend/auth, and host-api failures map to stable typed outcomes before egress.
- Configured credential plus approved write still dispatches through `HostRuntime` and `RuntimeHttpEgress`.
- A write with no usable Google credential authenticates first; after callback resume it reaches the normal approval gate, then dispatches only after approval.

## Follow-Up Issues

- #4112 WebUI v2 GSuite OAuth prompt wiring and browser E2E for GSuite OAuth + approval: render `AuthChallenge::OAuthUrl` / auth-required projection, open the OAuth URL, display completion/failure state, submit any required gate resolution, install Gmail/Calendar, trigger auth, complete fake provider callback, resume, hit write approval, and verify dispatch.
- #4113 Google token refresh and account health: refresh expired access tokens, mark refresh failures, support reauthorize-required recovery, and test no raw-token leakage.
- #3968 Live Google harness: optional env-gated Calendar/Gmail live tests using seeded refresh token/client config, skipped by default.

## Assumptions

- OAuth ownership stays in `ironclaw_auth`; no new product-facing `ironclaw_oauth` crate.
- GSuite stays in `ironclaw_first_party_extensions`; no resurrection of the older `ironclaw_native_extensions` plan.
- This slice is backend/service-level only. WebUI and browser E2E are explicit follow-ups.
- Runtime env vars alone should not silently create credentials; they only configure the OAuth client used when a real auth flow is started.
