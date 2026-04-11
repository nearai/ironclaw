# SSO/OIDC User Authentication — Implementation Plan

**Issue**: #1610
**Depends on**: #1605 (DB-backed user management) — MERGED (V14 migration, `UserStore` trait, admin/profile/token handlers all present)
**Priority**: P1

## 1. Current State Assessment

### Existing Auth System (`src/channels/web/auth.rs`)

The gateway already supports three auth mechanisms, tried in order:

1. **Env-var bearer tokens** (`MultiAuthState`) — SHA-256 hashed, constant-time comparison, maps tokens to `UserIdentity { user_id, role, workspace_read_scopes }`.
2. **DB-backed bearer tokens** (`DbAuthenticator`) — LRU-cached (60s TTL, 1024 entries), queries `api_tokens` + `users` tables, records usage.
3. **OIDC JWT validation** (`OidcState`) — Reads JWT from a configurable header (default: `x-amzn-oidc-data`), fetches signing keys from JWKS endpoint, verifies signature + claims. Designed for reverse-proxy setups (AWS ALB + Okta/Cognito). The `sub` claim becomes the `user_id`. **No user auto-provisioning** — the OIDC path creates an anonymous `UserIdentity` with role `member` but never creates a `UserRecord` in the database.

### Existing OAuth Code (`src/cli/oauth_defaults.rs`)

This is **extension OAuth** — for authenticating WASM tool access to third-party APIs (Google Drive, Gmail, etc.). Completely separate concern from user login. Uses a local callback listener on a fixed port for desktop OAuth flows. Not relevant to SSO user auth.

### Existing User Management (#1605 — Merged)

- **Tables**: `users` (id, email, display_name, status, role, metadata) and `api_tokens` (id, user_id, token_hash, token_prefix, name, expires_at, revoked_at) — both PostgreSQL (V14 migration) and libSQL.
- **Trait**: `UserStore` in `src/db/mod.rs` — `create_user`, `get_user`, `get_user_by_email`, `list_users`, `update_user_status/role/profile`, `record_login`, `create_api_token`, `list_api_tokens`, `revoke_api_token`, `authenticate_token`, `create_user_with_token`, `has_any_users`, `delete_user`.
- **Handlers**: `src/channels/web/handlers/users.rs` (admin CRUD), `tokens.rs` (self-service token management), profile endpoint.
- **Auth extractors**: `AuthenticatedUser`, `AdminUser` — both pull `UserIdentity` from request extensions.

### What's Missing for SSO

1. **No browser login flow** — The OIDC path only validates JWTs from a reverse proxy header. There's no interactive OAuth 2.0 Authorization Code flow where a browser user clicks "Sign in with Google" and gets redirected.
2. **No session/cookie management** — Auth is stateless (bearer token or JWT per request). Browser SSO needs server-side sessions with HTTP-only cookies.
3. **No OIDC discovery** — The current `GatewayOidcConfig` requires a manually configured JWKS URL. Full OIDC providers need `.well-known/openid-configuration` discovery.
4. **No user auto-provisioning** — OIDC-authenticated users get a transient `UserIdentity` but no `UserRecord`. SSO needs to auto-create users on first login.
5. **No multi-provider support** — Only one OIDC config can be active. Need to support Google + Okta + generic OIDC simultaneously.

## 2. Architecture Design

### Design Principles

- **Extend, don't replace**: The existing bearer-token and OIDC-JWT-header paths stay as-is. SSO adds a new browser-session auth path alongside them.
- **Generic provider abstraction**: Model all providers (Google, Okta, generic OIDC) through a single `SsoProvider` trait/enum, not hardcoded per-provider logic.
- **Session = DB row + signed cookie**: Server-side sessions stored in the database, referenced by an HTTP-only signed cookie. No JWT-as-session (avoids revocation headaches).
- **Auto-provision on first login**: When an SSO user authenticates for the first time and their email isn't in `users`, create a `UserRecord` automatically with `role: member`.
- **Dual-backend**: All new tables support PostgreSQL and libSQL per project rules.

### Auth Flow Diagram

```
Browser                         Gateway                          IdP (Google/Okta)
  │                                │                                │
  ├─ GET /auth/login?provider=X ──►│                                │
  │                                ├─ Generate PKCE + state ────────┤
  │                                ├─ Store state in sso_auth_state │
  │  ◄─ 302 Redirect to IdP ──────┤                                │
  │                                                                 │
  ├─ User authenticates at IdP ──────────────────────────────────►  │
  │                                                                 │
  │  ◄─ 302 Redirect to /auth/callback?code=X&state=Y ─────────────┤
  │                                │                                │
  ├─ GET /auth/callback ──────────►│                                │
  │                                ├─ Verify state                  │
  │                                ├─ Exchange code for tokens ────►│
  │                                │  ◄─ id_token + access_token ───┤
  │                                ├─ Validate id_token (sig+claims)│
  │                                ├─ Auto-provision user if new    │
  │                                ├─ Create session in DB          │
  │  ◄─ Set-Cookie: session_id ────┤                                │
  │  ◄─ 302 Redirect to /  ───────┤                                │
  │                                │                                │
  ├─ GET /api/chat/send ──────────►│                                │
  │  (Cookie: session_id)          ├─ Lookup session in DB          │
  │                                ├─ Resolve UserIdentity          │
  │                                ├─ Insert into extensions        │
  │                                ├─ Next handler ─────────────►   │
```

## 3. New Configuration Types

### `src/config/channels.rs` — New `SsoConfig`

```rust
/// SSO/OIDC authentication configuration.
#[derive(Debug, Clone)]
pub struct SsoConfig {
    /// Whether SSO login is enabled.
    pub enabled: bool,
    /// Configured SSO providers.
    pub providers: Vec<SsoProviderConfig>,
    /// Session cookie name (default: `__ironclaw_session`).
    pub session_cookie_name: String,
    /// Session TTL in seconds (default: 86400 = 24h).
    pub session_ttl_secs: u64,
    /// HMAC secret for signing session cookies.
    /// Auto-generated if not set.
    pub session_secret: Option<String>,
    /// Whether to auto-create users on first SSO login (default: true).
    pub auto_provision: bool,
    /// Default role for auto-provisioned users (default: "member").
    pub auto_provision_role: String,
    /// Allowed email domains for auto-provisioning (empty = allow all).
    pub allowed_domains: Vec<String>,
    /// External URL of this gateway (for redirect_uri computation).
    /// Falls back to `http://{host}:{port}` if unset.
    pub external_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SsoProviderConfig {
    /// Provider identifier (e.g., "google", "okta", "custom-oidc").
    pub id: String,
    /// Display name for the login button.
    pub display_name: String,
    /// OIDC discovery URL (e.g., `https://accounts.google.com`).
    /// The `.well-known/openid-configuration` suffix is appended automatically.
    pub issuer_url: String,
    /// OAuth 2.0 client ID.
    pub client_id: String,
    /// OAuth 2.0 client secret.
    pub client_secret: String,
    /// Additional scopes beyond `openid email profile`.
    pub extra_scopes: Vec<String>,
    /// Whether this provider is trusted to return `email_verified == true`
    /// correctly on every id_token. MUST default to `false`. See §7.1 —
    /// providers with `false` are excluded from email-based auto-linking
    /// and domain-based auto-provisioning; they may only be used via
    /// explicit invite-based flows or pre-existing (provider_id, sub)
    /// identity links.
    pub emits_verified_email_claim: bool,
}
```

### Environment Variables

```
SSO_ENABLED=true
SSO_SESSION_SECRET=<random-hex>
SSO_SESSION_TTL=86400
SSO_AUTO_PROVISION=true
SSO_AUTO_PROVISION_ROLE=member
SSO_ALLOWED_DOMAINS=example.com,corp.example.com
SSO_EXTERNAL_URL=https://my-ironclaw.example.com

# Per-provider (numbered, 1-indexed):
SSO_PROVIDER_1_ID=google
SSO_PROVIDER_1_DISPLAY_NAME=Google Workspace
SSO_PROVIDER_1_ISSUER_URL=https://accounts.google.com
SSO_PROVIDER_1_CLIENT_ID=...
SSO_PROVIDER_1_CLIENT_SECRET=...
SSO_PROVIDER_1_EXTRA_SCOPES=
# Safe: Google reliably returns email_verified for Workspace and consumer accounts.
SSO_PROVIDER_1_EMITS_VERIFIED_EMAIL_CLAIM=true

SSO_PROVIDER_2_ID=okta
SSO_PROVIDER_2_DISPLAY_NAME=Okta SSO
SSO_PROVIDER_2_ISSUER_URL=https://my-org.okta.com
SSO_PROVIDER_2_CLIENT_ID=...
SSO_PROVIDER_2_CLIENT_SECRET=...
# Okta returns email_verified only when the org requires email verification.
# Default false; set to true only after auditing your Okta org's verification policy.
SSO_PROVIDER_2_EMITS_VERIFIED_EMAIL_CLAIM=false
```

## 4. New Database Tables / Migrations

### PostgreSQL: `migrations/V15__sso_sessions.sql`

```sql
-- SSO sessions for browser-based authentication.
CREATE TABLE sso_sessions (
    id TEXT PRIMARY KEY,                          -- random session ID (32 hex chars)
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,                    -- which SSO provider was used
    id_token_sub TEXT NOT NULL,                   -- OIDC `sub` claim for this session
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ip_address TEXT,                              -- client IP at session creation
    user_agent TEXT                               -- browser user-agent at session creation
);
CREATE INDEX idx_sso_sessions_user ON sso_sessions(user_id);
CREATE INDEX idx_sso_sessions_expires ON sso_sessions(expires_at);

-- OIDC auth state for CSRF protection during the login flow.
-- Rows are ephemeral (auto-expire after 10 minutes).
CREATE TABLE sso_auth_state (
    state TEXT PRIMARY KEY,                       -- random nonce (CSRF token)
    provider_id TEXT NOT NULL,
    pkce_verifier TEXT NOT NULL,                  -- PKCE code_verifier
    redirect_after TEXT,                          -- where to redirect after login
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL               -- 10 minutes from creation
);
CREATE INDEX idx_sso_auth_state_expires ON sso_auth_state(expires_at);

-- SSO identity links: maps OIDC provider+sub to a local user.
-- Allows a single user to link multiple SSO providers.
CREATE TABLE sso_identity_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    oidc_sub TEXT NOT NULL,                       -- `sub` from the OIDC id_token
    oidc_email TEXT,                              -- `email` claim (for display)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider_id, oidc_sub)                -- one local user per provider+sub
);
CREATE INDEX idx_sso_identity_links_user ON sso_identity_links(user_id);
```

### libSQL: Add to `INCREMENTAL_MIGRATIONS` in `src/db/libsql_migrations.rs`

Same schema translated to libSQL dialect:
- `UUID` -> `TEXT`
- `TIMESTAMPTZ` -> `TEXT`
- `DEFAULT NOW()` -> `DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))`
- `gen_random_uuid()` -> omitted (generate in Rust)

## 5. New HTTP Routes

All under the **public** router (no auth required — these ARE the auth flow):

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/auth/providers` | `sso_providers_handler` | List available SSO providers (for login page) |
| GET | `/auth/login` | `sso_login_handler` | Initiate SSO flow — `?provider=google&redirect=/` |
| GET | `/auth/callback` | `sso_callback_handler` | OIDC callback — exchange code, create session |
| POST | `/auth/logout` | `sso_logout_handler` | Destroy session, clear cookie |
| GET | `/auth/session` | `sso_session_handler` | Check current session status (for frontend) |

**Note**: `/auth/callback` is separate from the existing `/oauth/callback` which handles extension OAuth. The namespaces are intentionally different (`/auth/` = user auth, `/oauth/` = extension auth).

## 6. Session Middleware Design

### Cookie Format

- Name: `__ironclaw_session` (configurable)
- Value: `{session_id}.{hmac_signature}` — HMAC-SHA256 signed with `SSO_SESSION_SECRET`
- Attributes: `HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={ttl}`

### Middleware Integration

Add session resolution as a **fourth auth path** in `auth_middleware`:

```
1. Bearer token (env-var) -> UserIdentity
2. Bearer token (DB) -> UserIdentity
3. OIDC JWT header -> UserIdentity
4. NEW: Session cookie -> DB lookup -> UserIdentity
5. 401
```

The session middleware:
1. Extracts `__ironclaw_session` cookie from request
2. Validates HMAC signature (rejects tampered cookies without DB hit)
3. Looks up session in `sso_sessions` (with LRU cache, similar to `DbAuthenticator`)
4. Checks `expires_at` and user status
5. Updates `last_accessed_at` (best-effort, non-blocking)
6. Inserts `UserIdentity` into request extensions

### `CombinedAuthState` Changes

```rust
pub struct CombinedAuthState {
    pub env_auth: MultiAuthState,
    pub db_auth: Option<DbAuthenticator>,
    pub oidc: Option<OidcState>,
    pub sso: Option<SsoAuthState>,  // NEW
}
```

`SsoAuthState` holds:
- `SsoConfig` (provider configs)
- Cached OIDC discovery documents per provider
- Session cache (LRU, similar to `DbAuthenticator`)
- `Arc<dyn Database>` for session/user lookups
- HMAC signing key for cookies

## 7. Auto-Provisioning Flow

When `/auth/callback` receives a valid id_token:

1. Extract `sub`, `email`, `email_verified`, `name` from claims
2. **Verified-email gate (MANDATORY, see §7.1)** — if the flow will touch email-based linking or domain-based auto-provisioning, enforce `email_verified == true` BEFORE step 3. Missing or `false` -> reject with a clear error page; do not fall through.
3. Check `sso_identity_links` for `(provider_id, sub)` match
4. **If linked**: Load user, check status, create session. (Provider+sub identity links are trust-on-first-use once established and do NOT require re-checking `email_verified` on every login, because the link itself was gated on a verified email at creation time.)
5. **If not linked**:
   a. Check `users` by email (may exist from admin-created invite) — **requires `email_verified == true`**
   b. **If user exists by email**: Create identity link, create session
   c. **If no user**: Auto-provision (if `SSO_AUTO_PROVISION=true`) — **requires `email_verified == true`**
      - Validate email domain against `SSO_ALLOWED_DOMAINS`
      - `create_user(UserRecord { id: uuid, email, display_name: name, role: auto_provision_role, status: "active" })`
      - Create `sso_identity_links` row
      - Create session
   d. **If auto-provision disabled and no user**: Return error page

### 7.1 Verified Email Requirement (MANDATORY)

**Threat model.** If the SSO flow links or provisions local accounts by email without checking `email_verified`, any OIDC provider (or provider configuration) that issues id_tokens containing an attacker-controlled, unverified email allows **silent account takeover** of the matching local user. Example: an attacker registers `victim@corp.example.com` at a misconfigured or permissive IdP that never verified ownership, logs in via SSO, and the gateway links the new identity to the existing `victim@corp.example.com` local account — granting full access to that user's data, memory, tools, and history. This is a well-known class of OIDC vulnerability and must be closed at the plan level, not patched later.

**Rules.**

1. **Claim required.** Every id_token used for a linking or provisioning branch MUST contain `email_verified: true` (boolean). Missing claim, `false`, `"false"` (string), `null`, or any non-boolean value MUST be treated as unverified.
2. **Hard reject, no fall-through.** If unverified, the callback handler MUST return an error page ("Your identity provider did not confirm ownership of this email address; contact your administrator") and MUST NOT create a session, MUST NOT create a user, and MUST NOT create an identity link. There is no "maybe link, warn the user" path.
3. **Applies to every email-keyed branch.** Steps 5(a), 5(b), 5(c) above are all gated. The only branch that does NOT require re-checking `email_verified` on each login is step 4 (existing provider+sub link), because the link was established under this same gate.
4. **Provider eligibility for email-based flows.** `SsoProviderConfig` gains a new field:
   ```rust
   /// Whether this provider is trusted to return `email_verified` correctly
   /// on every id_token. Providers set this to `false` MUST NOT participate
   /// in email-based auto-linking (7.5a) or domain-based auto-provisioning
   /// (7.5c). They may still authenticate existing users via an already-
   /// established (provider_id, sub) identity link, and may still be used
   /// via explicit invite-based flows where an admin pre-creates the user.
   pub emits_verified_email_claim: bool,
   ```
   - Default: `false` (fail-closed). Operators must opt in explicitly per provider.
   - Known-good defaults documented in `.env.example`:
     - **Google (`accounts.google.com`)**: reliably returns `email_verified` for Workspace and consumer accounts. Safe to set `true`.
     - **Okta**: returns `email_verified` when the Okta org requires email verification; operators MUST confirm their org's email verification policy before setting `true`.
     - **Generic OIDC / Keycloak / Auth0 / Authentik**: depends entirely on realm configuration. MUST default to `false`; operators set `true` only after auditing their realm's verification flow.
   - If `emits_verified_email_claim == false`, the callback handler treats all email-based linking/provisioning as disabled for that provider, regardless of the `email_verified` value in any incoming id_token. The provider is effectively **invite-only**: a user must already exist in `users` AND an identity link must already exist via a trusted path (admin creation, or a prior login from a provider that is trusted for email).
5. **Config validation at startup.** If `SSO_AUTO_PROVISION=true` and ANY configured provider has `emits_verified_email_claim == false`, gateway startup MUST log a warning naming the provider and noting that it is excluded from auto-provisioning. This prevents silent misconfiguration.
6. **Audit.** Every rejected-unverified-email attempt MUST be logged at `warn!` level with `provider_id`, `oidc_sub`, and the raw `email_verified` value observed, so operators can spot a misconfigured IdP or an active attack.

### 7.2 Redirect Parameter Safety (MANDATORY)

**Threat model.** The `/auth/login?provider=X&redirect=Y` endpoint persists `redirect_after` in `sso_auth_state` and consumes it in `/auth/callback` to send the browser to `Y` after successful login. If `Y` is not constrained to a same-origin relative path, the gateway becomes an **open-redirect sink**: an attacker crafts a phishing link `https://ironclaw.example.com/auth/login?provider=google&redirect=https://evil.example/` that looks legitimate (correct origin, real TLS cert, real login flow) but lands the user on an attacker-controlled page immediately after auth. Open redirects are also routinely chained into OAuth token theft, credential phishing, and SSRF probes.

**Rules.**

1. **Allowed shape: same-origin relative path only.** The only accepted form is a path beginning with a single `/`, optionally followed by `?query` and `#fragment`. Examples of accepted values: `/`, `/chat`, `/settings?tab=profile`, `/memory#recent`.
2. **Explicit rejects.** The following MUST be rejected (and replaced with the safe default `/`):
   - Any value not starting with `/`.
   - Values starting with `//` (protocol-relative URL — `//evil.com/path` is interpreted by browsers as `https://evil.com/path`).
   - Values starting with `/\` or containing a backslash anywhere before the first `/` or `?` (defeats naive prefix checks on some browser URL parsers).
   - Values containing `://`, `\\`, or any occurrence of a scheme (`http:`, `https:`, `javascript:`, `data:`, `file:`, `vbscript:`).
   - Values containing CR (`\r`), LF (`\n`), NUL (`\0`), or any other control character (header injection / log forging).
   - Values longer than 2048 bytes (defense-in-depth cap).
   - Values containing `..` path segments that escape the gateway root (e.g. `/../admin`). Normalization is performed server-side; any value whose normalized form does not still start with `/` is rejected.
3. **Where validation happens (BOTH sides, defense in depth).**
   - **Persist-time** (`sso_login_handler`, before writing to `sso_auth_state`): the raw query param is validated. Invalid -> stored as `/`. This prevents a poisoned value from ever reaching the database.
   - **Consume-time** (`sso_callback_handler`, after reading `sso_auth_state.redirect_after`): the stored value is re-validated before being used in the 302 `Location` header. Invalid -> `/`. This catches any value that somehow bypassed persist-time validation (schema change, direct DB write, migration artifact).
4. **No host-based allowlisting.** The plan deliberately does NOT offer a "trusted external redirect hosts" escape hatch. Same-origin relative paths are the entire universe of allowed redirects. If a future feature needs a cross-origin hop, it must be designed as its own audited mechanism, not bolted onto the login flow.
5. **Implementation location.** A single helper `fn sanitize_post_login_redirect(raw: &str) -> String` lives in `src/channels/web/sso.rs` and returns either the validated path or `"/"`. Both `sso_login_handler` and `sso_callback_handler` call it. The helper is pure and trivially unit-testable, but per the testing rule below the **authoritative** tests drive it through the real handlers.

## 8. Crate Recommendations

| Crate | Purpose | Notes |
|-------|---------|-------|
| `openidconnect` (0.7.x) | OIDC discovery, token exchange, id_token validation | Well-maintained, handles all OIDC complexity. Already uses `reqwest` (which is in Cargo.toml). Replaces the need for manual JWKS fetching in the SSO path. |
| `oauth2` (5.x) | Underlying OAuth 2.0 primitives | Pulled in by `openidconnect` automatically. Handles PKCE, authorization URLs, token exchange. |
| `hmac` + `sha2` | Cookie signing | `sha2` is already a dependency. Add `hmac` for HMAC-SHA256. |

**Not recommended**:
- `tower-sessions` / `axum-sessions` — Too opaque for our needs. We already have DB-backed auth patterns (`DbAuthenticator`). Rolling our own session table is simpler and gives full control over cache/eviction.
- `cookie` crate — Nice-to-have but not needed. Cookie parsing/serialization is simple enough to inline (Set-Cookie header formatting).

## 9. File-by-File Change List

### New Files

| File | Description |
|------|-------------|
| `src/channels/web/sso.rs` | Core SSO logic: `SsoAuthState`, OIDC discovery client, provider registry, login/callback/logout handlers, session cookie signing/verification, auto-provisioning |
| `src/channels/web/handlers/sso.rs` | HTTP handler functions (`sso_login_handler`, `sso_callback_handler`, `sso_logout_handler`, `sso_providers_handler`, `sso_session_handler`) |
| `src/db/libsql/sso.rs` | libSQL implementation of `SsoStore` |
| `migrations/V15__sso_sessions.sql` | PostgreSQL migration |

### Modified Files

| File | Changes |
|------|---------|
| `src/config/channels.rs` | Add `SsoConfig`, `SsoProviderConfig` structs. Add `sso: Option<SsoConfig>` to `GatewayConfig`. Parse `SSO_*` env vars. |
| `src/config/mod.rs` | Re-export new config types |
| `src/channels/web/mod.rs` | Add `pub mod sso;`. Add `with_sso()` builder method on `GatewayChannel`. Add `sso` field to `CombinedAuthState` and `GatewayState`. Wire up in `rebuild_state()`. |
| `src/channels/web/auth.rs` | Add session cookie auth as 4th path in `auth_middleware`. Add `SsoAuthState` to `CombinedAuthState`. Add cookie extraction helper. Add `/auth/*` paths to CORS allowed origins. |
| `src/channels/web/server.rs` | Register `/auth/*` routes in the `public` router. Add `sso_config` field to `GatewayState`. Import new handler functions. |
| `src/channels/web/handlers/mod.rs` | Add `pub mod sso;` |
| `src/db/mod.rs` | Add `SsoStore` sub-trait with methods: `create_sso_session`, `get_sso_session`, `delete_sso_session`, `cleanup_expired_sso_sessions`, `create_sso_auth_state`, `consume_sso_auth_state`, `create_sso_identity_link`, `get_sso_identity_link`, `list_sso_identity_links`. Add to `Database` supertrait bounds. |
| `src/db/postgres.rs` | Implement `SsoStore` for PostgreSQL backend |
| `src/db/libsql/mod.rs` | Add `mod sso;` |
| `src/db/libsql_migrations.rs` | Add migration 15 to `INCREMENTAL_MIGRATIONS` for the three new tables |
| `src/app.rs` | Parse `SsoConfig`, call `with_sso()` on `GatewayChannel` during startup |
| `src/channels/web/static/app.js` | Add login page / SSO provider buttons when session cookie is missing. Handle `/auth/session` check. Add logout button. |
| `src/channels/web/static/style.css` | Login page styling |
| `.env.example` | Document all `SSO_*` environment variables |
| `Cargo.toml` | Add `openidconnect`, `hmac` dependencies |

## 10. Testing Strategy

**Binding rule.** Per `.claude/rules/testing.md` ("Test Through the Caller, Not Just the Helper"), the two security-critical flows below — verified-email gating and post-login redirect sanitization — MUST be covered by tests that drive the **real handlers and the real axum `auth_middleware` stack**, not by unit tests on isolated predicates or validators. Helper-only tests are *supplementary* only; they do not count as regression coverage for these risks. If this plan's earlier draft implied helper-only coverage was sufficient, that guidance is replaced by this section.

Rationale: both `email_verified` enforcement and `sanitize_post_login_redirect` sit behind a handler with wrapping logic (cookie parsing, state lookup, token exchange, session creation, 302 construction). A unit test that calls the helper in isolation cannot catch regressions where the call site stops invoking the helper, invokes it with the wrong argument, invokes it at persist-time but not consume-time, or reads a different field of the claims struct than the helper validates. Every such gap is a silent re-opening of the exact vulnerabilities this plan is closing.

### 10.1 Verified-Email Enforcement — Authoritative Tests (integration tier)

Location: `tests/sso_verified_email.rs` (new, gated on `#[cfg(feature = "integration")]`).

Each test constructs the gateway's real axum `Router` via the same factory used in production (`src/channels/web/server.rs::build_router` or equivalent), wires it to an in-memory or testcontainers DB backend, and drives `/auth/callback` end-to-end with a mocked OIDC token endpoint. Assertions target HTTP status, `Set-Cookie` presence, and `users` / `sso_identity_links` DB state AFTER the request completes.

| Case | Setup | Expected outcome |
|------|-------|------------------|
| Verified, new user, provider trusted | `emits_verified_email_claim=true`, id_token has `email_verified: true`, no existing user | 302 to `/`, session cookie set, new row in `users` and `sso_identity_links` |
| Verified, existing user by email, provider trusted | Same as above but user pre-exists with matching email | 302 to `/`, session cookie set, NO new user row, new `sso_identity_links` row |
| Verified, existing provider+sub link | Identity link already exists | 302 to `/`, session cookie set, no new rows |
| **Unverified (`false`)** | id_token has `email_verified: false`, no existing link | 4xx error page, NO cookie, NO new `users` row, NO new `sso_identity_links` row |
| **Claim missing entirely** | id_token omits `email_verified` | 4xx error page, NO cookie, NO new rows |
| **Claim is string `"false"`** | id_token has `email_verified: "false"` | 4xx error page, NO cookie, NO new rows (treats non-boolean as unverified) |
| **Claim is string `"true"`** | id_token has `email_verified: "true"` | 4xx error page, NO cookie, NO new rows (strict boolean-only acceptance) |
| **Claim is `null`** | id_token has `email_verified: null` | 4xx error page, NO cookie, NO new rows |
| **Provider not trusted, verified claim present** | `emits_verified_email_claim=false`, id_token has `email_verified: true`, no existing link | 4xx error page, NO cookie, NO new rows (untrusted providers are invite-only regardless of claim value) |
| **Provider not trusted, existing link** | `emits_verified_email_claim=false`, existing `(provider_id, sub)` link | 302 to `/`, session cookie set (pre-existing link is still honored) |
| **Audit log** | Any unverified rejection above | `warn!` record with `provider_id`, `oidc_sub`, observed `email_verified` value |

Every row above is driven through `tower::ServiceExt::oneshot(router, request)` against the real router; none of them call `validate_email_verified(...)` or equivalent in isolation.

### 10.2 Redirect Sanitization — Authoritative Tests (integration tier)

Location: `tests/sso_redirect_safety.rs` (new, gated on `#[cfg(feature = "integration")]`).

Each test drives the full login + callback sequence through the real router. Persist-time tests issue `GET /auth/login?provider=...&redirect=<payload>` and then inspect the `sso_auth_state.redirect_after` column directly. Consume-time tests seed `sso_auth_state` with a poisoned value and then drive `/auth/callback`, asserting the `Location` header on the 302 response.

| Payload | Persist-time expectation | Consume-time expectation |
|---------|--------------------------|--------------------------|
| `/` | Stored as `/` | `Location: /` |
| `/chat` | Stored as `/chat` | `Location: /chat` |
| `/settings?tab=profile` | Stored verbatim | `Location: /settings?tab=profile` |
| `/memory#recent` | Stored verbatim | `Location: /memory#recent` |
| `//evil.example/path` (protocol-relative) | Stored as `/` | `Location: /` |
| `///evil.example` | Stored as `/` | `Location: /` |
| `https://evil.example/` | Stored as `/` | `Location: /` |
| `http://evil.example/` | Stored as `/` | `Location: /` |
| `javascript:alert(1)` | Stored as `/` | `Location: /` |
| `data:text/html,...` | Stored as `/` | `Location: /` |
| `/\evil.example` (backslash trick) | Stored as `/` | `Location: /` |
| `\\evil.example` | Stored as `/` | `Location: /` |
| `/..//evil.example` (path traversal) | Stored as `/` | `Location: /` |
| `/../admin` | Stored as `/` | `Location: /` |
| `/path%0d%0aLocation:%20https://evil` (CRLF injection) | Stored as `/` | `Location: /` |
| Empty string | Stored as `/` | `Location: /` |
| 4 KB garbage | Stored as `/` | `Location: /` |
| `chat` (no leading slash) | Stored as `/` | `Location: /` |

**Consume-time poison test (critical).** One test writes a raw `https://evil.example/` directly into `sso_auth_state.redirect_after` via the `SsoStore` trait (simulating a migration artifact, direct DB write, or a bug that bypassed persist-time validation), then drives `/auth/callback` through the real router and asserts `Location: /`. This is the test that proves consume-time re-validation is wired up; without it, a regression that drops consume-time validation would still pass all persist-time cases.

### 10.3 Middleware Routing Through the Real Auth Stack

Location: `tests/sso_middleware.rs` (new, gated on `#[cfg(feature = "integration")]`).

| Test | What it covers |
|------|----------------|
| Session cookie -> `UserIdentity` in handler extensions | Drive an authenticated request through the real `auth_middleware` chain; assert the downstream handler observes the expected `UserIdentity` |
| Priority ordering | Request with BOTH bearer token and session cookie — bearer wins per the documented order |
| Tampered cookie rejection | Cookie with valid session-ID prefix but invalid HMAC — 401, no DB hit |
| Expired session | Session row with `expires_at` in the past — 401 |
| Suspended user | Valid session but `users.status = 'suspended'` — 401 |
| Deleted session | Session row removed mid-request — 401, no stale cache hit after TTL |

All tests construct the router via the production factory and exercise the middleware as an HTTP-facing layer, not by calling helper functions.

### 10.4 Supplementary Unit Tests (NOT a substitute for the above)

These are fine to have as fast feedback for helper internals, but the test suite MUST still contain the integration-tier coverage in 10.1–10.3. If a PR adds only unit coverage for these helpers without the corresponding integration-tier test, reviewers must block it.

| Test | Location | Purpose |
|------|----------|---------|
| Cookie HMAC round-trip | `src/channels/web/sso.rs` | Fast feedback on signing primitive |
| Session lookup cache eviction | `src/channels/web/sso.rs` | Fast feedback on LRU behavior |
| `sanitize_post_login_redirect` table-driven | `src/channels/web/sso.rs` | Fast feedback on the pure helper — same payload table as 10.2 but invoked directly |
| `email_verified` claim parser | `src/channels/web/sso.rs` | Fast feedback on boolean-strict parsing |
| Config parsing | `src/config/channels.rs` | Multi-provider env-var parsing, including `emits_verified_email_claim` default=false |

### 10.5 Other Integration Tests (not security-critical)

| Test | What it covers |
|------|----------------|
| `SsoStore` round-trip (both backends) | Create/get/delete sessions, auth state CSRF nonce lifecycle, identity links, expired session cleanup |
| Concurrent session handling | Multiple sessions for same user, session invalidation on suspend |
| Startup warning for misconfigured providers | `SSO_AUTO_PROVISION=true` + a provider with `emits_verified_email_claim=false` logs a warning naming the provider |

### 10.6 E2E Tests (`tests/e2e/`)

| Scenario | What it covers |
|----------|----------------|
| `test_sso_login_redirect.py` | `/auth/login?provider=google` returns 302 to Google with correct params |
| `test_sso_callback_flow.py` | Mock IdP callback with verified email, session cookie set, redirect to `/` |
| `test_sso_callback_unverified_email.py` | Mock IdP callback with `email_verified: false` — error page, no cookie |
| `test_sso_logout.py` | POST `/auth/logout` clears cookie, subsequent requests are 401 |
| `test_sso_auto_provision.py` | First-time SSO login with verified email creates user + identity link |
| `test_sso_domain_restriction.py` | Rejected email domain returns error, not a user |
| `test_sso_redirect_open_redirect.py` | `?redirect=https://evil.example` lands on `/` after callback, not the attacker URL |

### Manual Testing

- Google: Register OAuth 2.0 credentials in Google Cloud Console, set authorized redirect to `http://localhost:8080/auth/callback`
- Okta: Create OIDC app in Okta admin, configure redirect URI
- Generic: Any OIDC-compliant provider (Keycloak, Auth0, Authentik)

## 11. Security Considerations

1. **PKCE required** — All authorization code flows use PKCE (`S256` method) to prevent code interception attacks.
2. **State parameter** — Random nonce stored in `sso_auth_state` with 10-minute TTL for CSRF protection. Consumed on use (single-use).
3. **Cookie security** — `HttpOnly; Secure; SameSite=Lax`. HMAC-signed to prevent forgery. Session ID is 32 random hex chars (128 bits entropy).
4. **Session revocation** — Deleting the session row immediately invalidates it. Cache TTL ensures max 60s stale window (same as bearer token cache).
5. **Domain allowlisting** — `SSO_ALLOWED_DOMAINS` restricts which email domains can auto-provision, preventing unauthorized signups.
6. **Existing auth unaffected** — Bearer tokens and OIDC JWT header auth continue to work. SSO is purely additive.
7. **No token storage** — OAuth access/refresh tokens from the IdP are NOT stored. We only need the id_token for identity verification. (If future features need API access to the IdP, store tokens in the `secrets` table with AES-256-GCM encryption.)
8. **Verified email required for linking/provisioning** — see §7.1. Every email-keyed account lookup or creation is gated on `email_verified == true` (strict boolean) AND a per-provider `emits_verified_email_claim` trust flag. Providers default to untrusted; operators opt in per-provider in config. Closes the account-takeover vector where a permissive or misconfigured IdP could emit an attacker-controlled unverified email matching an existing local user.
9. **Same-origin-only post-login redirect** — see §7.2. The `redirect` query parameter on `/auth/login` and the persisted `redirect_after` column in `sso_auth_state` are constrained to same-origin relative paths (must start with a single `/`, no `//`, no scheme, no backslash tricks, no CRLF, no path-traversal escapes). Validation runs at BOTH persist-time and consume-time; invalid values fall back to `/`. Closes the open-redirect sink that would otherwise let attackers use the gateway as a phishing springboard.

## 12. Migration / Rollout Plan

1. **Phase 1**: Implement `SsoStore` DB trait + migrations (can merge independently)
2. **Phase 2**: Implement SSO handlers + session middleware (feature-flagged behind `SSO_ENABLED`)
3. **Phase 3**: Frontend login page + session management in `app.js`
4. **Phase 4**: Documentation + `.env.example` updates

SSO is fully opt-in. Existing deployments are unaffected until `SSO_ENABLED=true` is set.

## 13. Open Questions

1. **Should SSO sessions extend on activity?** Sliding window (update `expires_at` on each request) vs. fixed TTL. Recommend: sliding window with `SSO_SESSION_TTL` as the inactivity timeout, capped at 7 days absolute max.
2. **Admin-only SSO?** Should there be a way to restrict SSO to specific roles, or is domain allowlisting sufficient?
3. **Group/role mapping from IdP claims?** Some organizations want the IdP to dictate `admin` vs `member` role via custom claims (e.g., Okta groups). Defer to a follow-up issue or support via `metadata` extensibility.
4. **Should the existing `GatewayOidcConfig` (reverse-proxy OIDC) be merged into the new SSO system?** Recommend keeping them separate — the existing OIDC path is for infrastructure (ALB injects JWT), while SSO is for interactive browser login. They serve different deployment models.
