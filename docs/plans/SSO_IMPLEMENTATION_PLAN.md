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

SSO_PROVIDER_2_ID=okta
SSO_PROVIDER_2_DISPLAY_NAME=Okta SSO
SSO_PROVIDER_2_ISSUER_URL=https://my-org.okta.com
SSO_PROVIDER_2_CLIENT_ID=...
SSO_PROVIDER_2_CLIENT_SECRET=...
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

1. Extract `sub`, `email`, `name` from claims
2. Check `sso_identity_links` for `(provider_id, sub)` match
3. **If linked**: Load user, check status, create session
4. **If not linked**:
   a. Check `users` by email (may exist from admin-created invite)
   b. **If user exists by email**: Create identity link, create session
   c. **If no user**: Auto-provision (if `SSO_AUTO_PROVISION=true`)
      - Validate email domain against `SSO_ALLOWED_DOMAINS`
      - `create_user(UserRecord { id: uuid, email, display_name: name, role: auto_provision_role, status: "active" })`
      - Create `sso_identity_links` row
      - Create session
   d. **If auto-provision disabled and no user**: Return error page

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

### Unit Tests (`cargo test`)

| Test | Location | What it covers |
|------|----------|----------------|
| Cookie signing/verification | `src/channels/web/sso.rs` | HMAC round-trip, tampered cookie rejection, expired cookie handling |
| Session lookup cache | `src/channels/web/sso.rs` | Cache hit, TTL expiry, cache eviction |
| Auth middleware with session cookie | `src/channels/web/auth.rs` | Session cookie path in `auth_middleware`, priority ordering (bearer > OIDC > session) |
| Domain allowlist validation | `src/channels/web/sso.rs` | Email domain filtering for auto-provisioning |
| Config parsing | `src/config/channels.rs` | Multi-provider config parsing from env vars |

### Integration Tests (`cargo test --features integration`)

| Test | What it covers |
|------|----------------|
| `SsoStore` round-trip (both backends) | Create/get/delete sessions, auth state CSRF, identity links, expired session cleanup |
| Auto-provisioning flow | New user creation via SSO, linking to existing user by email, domain rejection |
| Session middleware end-to-end | Axum test app with session cookie -> handler receives correct `UserIdentity` |
| Concurrent session handling | Multiple sessions for same user, session invalidation on suspend |

### E2E Tests (`tests/e2e/`)

| Scenario | What it covers |
|----------|----------------|
| `test_sso_login_redirect.py` | `/auth/login?provider=google` returns 302 to Google with correct params |
| `test_sso_callback_flow.py` | Mock IdP callback, verify session cookie set, redirect to `/` |
| `test_sso_logout.py` | POST `/auth/logout` clears cookie, subsequent requests are 401 |
| `test_sso_auto_provision.py` | First-time SSO login creates user + identity link |
| `test_sso_domain_restriction.py` | Rejected email domain returns error, not a user |

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
