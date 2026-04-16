# User Management API

DB-backed user management for multi-tenant IronClaw deployments. Covers admin user CRUD, per-user secrets provisioning, self-service profile, API token management, and usage reporting.

## Authentication

All endpoints require `Authorization: Bearer <token>`. Tokens are either:
- **Env-var tokens** — configured via `GATEWAY_AUTH_TOKEN` (single-user) at startup
- **DB-backed tokens** — created via `POST /api/tokens` or `POST /api/admin/users`

DB tokens are SHA-256 hashed at rest; plaintext is returned exactly once at creation time.

Auth is cached in a bounded LRU (1024 entries, 60s TTL). Suspending a user or revoking a token may take up to 60s to take effect.

## Roles

| Role | Scope |
|------|-------|
| `admin` | Full access to all endpoints |
| `member` | Self-service profile + own token management only |

Endpoints marked **Admin** return `403 Forbidden` for `member` role.

---

## Admin: Users

### POST /api/admin/users

Create a new user. Returns the user record and a one-time plaintext API token.

**Auth:** Admin

**Request body:**

```json
{
  "display_name": "Alice Smith",
  "email": "alice@example.com",
  "role": "member"
}
```

| Field | Type | Required | Default | Notes |
|-------|------|----------|---------|-------|
| `display_name` | string | yes | | |
| `email` | string | no | `null` | Must be unique if provided |
| `role` | string | no | `"member"` | `"admin"` or `"member"` |

**Response:** `200 OK`

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "alice@example.com",
  "display_name": "Alice Smith",
  "status": "active",
  "role": "member",
  "token": "a1b2c3d4e5f6...64-char hex...",
  "created_at": "2026-03-25T12:00:00+00:00",
  "created_by": "admin-user-id"
}
```

The `token` field is the plaintext API token. It is shown **only once** — store it securely.

**Errors:** `400` (missing display_name, invalid role), `403` (not admin), `503` (no database)

---

### GET /api/admin/users

List all users.

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "users": [
    {
      "id": "550e8400-...",
      "email": "alice@example.com",
      "display_name": "Alice Smith",
      "status": "active",
      "role": "member",
      "created_at": "2026-03-25T12:00:00+00:00",
      "updated_at": "2026-03-25T12:00:00+00:00",
      "last_login_at": "2026-03-25T14:30:00+00:00",
      "created_by": "admin-user-id"
    }
  ]
}
```

---

### GET /api/admin/users/{id}

Get a single user by ID.

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "id": "550e8400-...",
  "email": "alice@example.com",
  "display_name": "Alice Smith",
  "status": "active",
  "role": "member",
  "created_at": "2026-03-25T12:00:00+00:00",
  "updated_at": "2026-03-25T12:00:00+00:00",
  "last_login_at": "2026-03-25T14:30:00+00:00",
  "created_by": "admin-user-id",
  "metadata": {}
}
```

**Errors:** `404` (user not found), `403` (not admin)

---

### PATCH /api/admin/users/{id}

Update a user's display name and/or metadata. Omitted fields are left unchanged.

**Auth:** Admin

**Request body:**

```json
{
  "display_name": "Alice Johnson",
  "metadata": {"department": "engineering"}
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `display_name` | string | no | |
| `role` | string | no | `"admin"` or `"member"` |
| `metadata` | object | no | Replaces entire metadata object (full replacement; keys not included are removed) |

**Response:** `200 OK` — returns the full updated user record (same shape as GET detail, without `last_login_at`/`created_by`).

**Errors:** `404` (user not found), `403` (not admin)

---

### POST /api/admin/users/{id}/suspend

Suspend a user. Suspended users cannot authenticate (DB auth checks user status).

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "id": "550e8400-...",
  "status": "suspended"
}
```

**Errors:** `404` (user not found), `403` (not admin)

---

### POST /api/admin/users/{id}/activate

Re-activate a suspended user.

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "id": "550e8400-...",
  "status": "active"
}
```

**Errors:** `404` (user not found), `403` (not admin)

---

### DELETE /api/admin/users/{id}

Permanently delete a user and all associated data (tokens, jobs, conversations, memory, routines, settings, secrets).

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "id": "550e8400-...",
  "deleted": true
}
```

**Errors:** `404` (user not found), `403` (not admin)

**Cascade:** Deletes from `api_tokens`, `agent_jobs`, `conversations`, `memory_documents`, `routines`, `secrets`, `settings`, `wasm_tools`, and related tables. On PostgreSQL this uses FK cascades; on libSQL it uses explicit deletes.

---

## Admin: Per-User Secrets

Provision secrets on behalf of individual users. The primary use case is an application backend (acting as admin) that configures per-user credentials so each user's IronClaw agent can call back to external services.

Secrets are encrypted at rest with AES-256-GCM using a per-secret HKDF-derived key. Plaintext values are **never returned** by any endpoint — they can only be used by the agent's tool system at runtime.

### PUT /api/admin/users/{user_id}/secrets/{name}

Create or update a secret for the specified user. If a secret with the same name already exists, it is overwritten.

**Auth:** Admin

**Path parameters:**

| Param | Type | Notes |
|-------|------|-------|
| `user_id` | string | The user's ID |
| `name` | string | Secret name (normalized to lowercase) |

**Request body:**

```json
{
  "value": "sk-live-abc123...",
  "provider": "my-app-backend",
  "expires_in_days": 90
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `value` | string | yes | The secret value (encrypted at rest, never returned) |
| `provider` | string | no | Tag for grouping (e.g. `"stripe"`, `"my-app"`) |
| `expires_in_days` | integer | no | Auto-expire after N days; `null` = never |

**Response:** `200 OK`

```json
{
  "user_id": "550e8400-...",
  "name": "my_app_callback_token",
  "status": "created"
}
```

**Errors:** `400` (missing value), `403` (not admin), `503` (secrets store not available)

**Example — application backend provisioning a callback token:**

```bash
# Admin creates a user
curl -X POST https://ironclaw.example.com/api/admin/users \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{"display_name": "Alice", "role": "member"}'
# Response includes: {"id": "alice-uuid", "token": "alice-bearer-token", ...}

# Admin provisions a per-user callback secret
curl -X PUT https://ironclaw.example.com/api/admin/users/alice-uuid/secrets/app_callback_token \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{"value": "per-user-jwt-for-alice", "provider": "my-app"}'

# Now Alice's IronClaw agent can use the "app_callback_token" secret
# when calling tools that need to authenticate back to the app backend.
```

---

### GET /api/admin/users/{user_id}/secrets

List a user's secrets. Returns names and providers only — **never values or hashes**.

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "user_id": "550e8400-...",
  "secrets": [
    {"name": "app_callback_token", "provider": "my-app"},
    {"name": "openai_api_key", "provider": "openai"}
  ]
}
```

---

### DELETE /api/admin/users/{user_id}/secrets/{name}

Delete a specific secret for a user.

**Auth:** Admin

**Response:** `200 OK`

```json
{
  "user_id": "550e8400-...",
  "name": "app_callback_token",
  "deleted": true
}
```

**Errors:** `404` (secret not found), `403` (not admin), `503` (secrets store not available)

---

## Admin: Usage

### GET /api/admin/usage

Per-user LLM usage statistics aggregated from `llm_calls` via `agent_jobs.user_id`.

**Auth:** Admin

**Query parameters:**

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `user_id` | string | all users | Filter to a single user |
| `period` | string | `"day"` | `"day"` (24h), `"week"` (7d), or `"month"` (30d) |

**Response:** `200 OK`

```json
{
  "period": "week",
  "since": "2026-03-18T12:00:00+00:00",
  "usage": [
    {
      "user_id": "alice-id",
      "model": "claude-sonnet-4-5-20250514",
      "call_count": 42,
      "input_tokens": 150000,
      "output_tokens": 30000,
      "total_cost": "1.23"
    }
  ]
}
```

---

## Self-Service: Profile

### GET /api/profile

Get the authenticated user's own profile.

**Auth:** Any authenticated user

**Response:** `200 OK`

```json
{
  "id": "550e8400-...",
  "email": "alice@example.com",
  "display_name": "Alice Smith",
  "status": "active",
  "role": "member",
  "created_at": "2026-03-25T12:00:00+00:00",
  "last_login_at": "2026-03-25T14:30:00+00:00"
}
```

---

### PATCH /api/profile

Update the authenticated user's own display name and/or metadata.

**Auth:** Any authenticated user

**Request body:**

```json
{
  "display_name": "Alice Johnson",
  "metadata": {"theme": "dark"}
}
```

**Response:** `200 OK`

```json
{
  "id": "550e8400-...",
  "display_name": "Alice Johnson",
  "updated": true
}
```

---

## Self-Service: Tokens

### POST /api/tokens

Create a new API token for the authenticated user. Admins can optionally create tokens for other users by including `user_id`.

**Auth:** Any authenticated user

**Request body:**

```json
{
  "name": "CI pipeline",
  "expires_in_days": 90,
  "user_id": "other-user-id"
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `name` | string | yes | Human-readable label |
| `expires_in_days` | integer | no | `null` = never expires |
| `user_id` | string | no | Admin-only; create token for another user |

**Response:** `200 OK`

```json
{
  "token": "a1b2c3d4...64-char hex...",
  "id": "token-uuid",
  "name": "CI pipeline",
  "token_prefix": "a1b2c3d4",
  "expires_at": "2026-06-23T12:00:00+00:00",
  "created_at": "2026-03-25T12:00:00+00:00"
}
```

The `token` field is shown **only once**.

---

### GET /api/tokens

List the authenticated user's tokens. Token hashes are never returned.

**Auth:** Any authenticated user

**Response:** `200 OK`

```json
{
  "tokens": [
    {
      "id": "token-uuid",
      "name": "CI pipeline",
      "token_prefix": "a1b2c3d4",
      "expires_at": "2026-06-23T12:00:00+00:00",
      "last_used_at": "2026-03-25T14:00:00+00:00",
      "created_at": "2026-03-25T12:00:00+00:00",
      "revoked_at": null
    }
  ]
}
```

---

### DELETE /api/tokens/{id}

Revoke one of the authenticated user's tokens. Users can only revoke their own tokens.

**Auth:** Any authenticated user

**Path:** `id` — UUID of the token to revoke

**Response:** `200 OK`

```json
{
  "status": "revoked",
  "id": "token-uuid"
}
```

**Errors:** `400` (invalid UUID), `404` (token not found or belongs to another user)

---

## Responses API

Routes through the full agent loop — tools, memory, safety, and server-side conversation state are all active. Compatible with any standard OpenAI SDK via `client.responses.create()`.

**Auth:** Any authenticated user (`Authorization: Bearer <token>`)

### POST /v1/responses

Create a response.

**Request body:**

```json
{
  "model": "claude-sonnet-4-5-20250514",
  "input": "What's a good time to send money to India?",
  "stream": true,
  "previous_response_id": null
}
```

| Field | Type | Notes |
|-------|------|-------|
| `model` | string | LLM model to use. Pass `"default"` to use the server-configured model |
| `input` | string or array | User message as a string, or a messages array (see below) |
| `stream` | boolean | `true` for SSE, `false` for blocking (120s timeout) |
| `previous_response_id` | string | Pass the previous `id` to continue a conversation thread |

**Messages array input:** `input` may be passed as an array instead of a string:

```json
{
  "model": "default",
  "input": [
    { "role": "user", "content": "What is 2+2? Reply with just the number." }
  ]
}
```

**Response (non-streaming):** `200 OK`

```json
{
  "id": "resp_<uuid>",
  "object": "response",
  "created_at": 1743000000,
  "model": "claude-sonnet-4-5-20250514",
  "status": "completed",
  "output": [
    {
      "type": "message",
      "id": "msg_<uuid>",
      "role": "assistant",
      "content": [{ "type": "output_text", "text": "Based on today's rate..." }]
    }
  ],
  "usage": {
    "input_tokens": 320,
    "output_tokens": 95,
    "total_tokens": 415
  }
}
```

**Streaming:** When `stream: true`, IronClaw returns SSE events:

| Event | Description |
|-------|-------------|
| `response.created` | Response object created, stream begins |
| `response.output_item.added` | New output item started (message or tool call) |
| `response.output_text.delta` | Text chunk |
| `response.output_item.done` | Output item finalized |
| `response.completed` | Full response done |
| `response.failed` | Error or tool approval required |

**Multi-turn:** Pass `previous_response_id` from the previous response's `id` to continue in the same thread. IronClaw decodes the thread UUID statelessly from the response ID — no lookup table required.

**Status values:** `completed` | `failed`

---

### Structured context

The `input` field accepts a structured `context` object alongside the user message. Use this to pass machine-readable state that the agent should act on, without relying solely on natural language.

**Request:**

```json
{
  "model": "claude-sonnet-4-5-20250514",
  "input": "Go ahead with the transfer",
  "previous_response_id": "resp_...",
  "x_context": {
    "notification_response": {
      "notification_id": "msg_123",
      "action": "approved",
      "original_signal": "convert_now",
      "score": 72,
      "rate": 85.42,
      "amount_usd": 1000
    }
  }
}
```

The agent receives the context prepended to the user message:

```
[Context: notification_response — notification_id: msg_123, action: approved,
original_signal: convert_now, score: 72, rate: 85.42, amount_usd: 1000]
Go ahead with the transfer
```

The `context` payload is persisted in message metadata for auditability.

---

### GET /v1/responses/{id}

Retrieve a historical response reconstructed from conversation messages in the database. Users can only retrieve their own responses.

**Auth:** Any authenticated user

**Response:** `200 OK` — same shape as the create response object above.

**Errors:** `400` (empty input), `401` (missing or invalid bearer token), `404` (response ID not found or not owned by caller), `500` (internal error)

---

## Error Format

All error responses return a plain text body with the error message and the corresponding HTTP status code:

| Code | Meaning |
|------|---------|
| `400` | Bad request (missing fields, invalid input) |
| `401` | Missing or invalid bearer token |
| `403` | Authenticated but insufficient role (member accessing admin endpoint) |
| `404` | Resource not found |
| `503` | Database or secrets store not available |
| `500` | Internal server error |

---

## Security Model

### Secrets Encryption

- **Algorithm:** AES-256-GCM with per-secret HKDF-SHA256 derived keys
- **Master key:** 32+ bytes, resolved from `SECRETS_MASTER_KEY` env var or OS keychain
- **Storage format:** `nonce (12B) || ciphertext || tag (16B)` in `encrypted_value` column
- **Per-secret salt:** 32 random bytes stored alongside the ciphertext
- **Zero-exposure:** Plaintext never appears in logs, debug output, API responses, or LLM conversations

### Auth Cache

- Bounded LRU cache (1024 entries max)
- 60-second TTL per entry
- Suspending a user or revoking a token takes up to 60s to propagate

---

## Database Schema

### users

| Column | Type (PG / libSQL) | Notes |
|--------|--------------------|-------|
| `id` | `TEXT` / `TEXT` | Primary key; typically UUID v4 strings (bootstrap admin may use a custom ID) |
| `email` | `TEXT UNIQUE` | Nullable |
| `display_name` | `TEXT NOT NULL` | |
| `status` | `TEXT NOT NULL` | `"active"` or `"suspended"` |
| `role` | `TEXT NOT NULL` | `"admin"` or `"member"` |
| `created_at` | `TIMESTAMPTZ` / `TEXT` | |
| `updated_at` | `TIMESTAMPTZ` / `TEXT` | |
| `last_login_at` | `TIMESTAMPTZ` / `TEXT` | Nullable |
| `created_by` | `TEXT` | Nullable, references `users.id` |
| `metadata` | `JSONB` / `TEXT` | Default `{}` |

### api_tokens

| Column | Type (PG / libSQL) | Notes |
|--------|--------------------|-------|
| `id` | `TEXT` / `TEXT` | Primary key; values are usually UUID v4 strings |
| `user_id` | `TEXT NOT NULL` | FK to `users.id` with cascading delete |
| `token_hash` | `BYTEA` / `BLOB` | SHA-256 of hex-encoded plaintext |
| `token_prefix` | `TEXT NOT NULL` | First 8 chars for identification |
| `name` | `TEXT NOT NULL` | Human-readable label |
| `expires_at` | `TIMESTAMPTZ` / `TEXT` | Nullable |
| `last_used_at` | `TIMESTAMPTZ` / `TEXT` | Nullable |
| `created_at` | `TIMESTAMPTZ` / `TEXT` | |
| `revoked_at` | `TIMESTAMPTZ` / `TEXT` | Nullable; set on revocation |

### secrets

| Column | Type (PG / libSQL) | Notes |
|--------|--------------------|-------|
| `id` | `UUID` / `TEXT` | Primary key |
| `user_id` | `TEXT NOT NULL` | Scoped to user |
| `name` | `TEXT NOT NULL` | Unique per user (lowercase normalized) |
| `encrypted_value` | `BYTEA` / `BLOB` | AES-256-GCM (nonce + ciphertext + tag) |
| `key_salt` | `BYTEA` / `BLOB` | Per-secret HKDF salt |
| `provider` | `TEXT` | Optional grouping tag |
| `expires_at` | `TIMESTAMPTZ` / `TEXT` | Nullable |
| `last_used_at` | `TIMESTAMPTZ` / `TEXT` | Audit: last injection time |
| `usage_count` | `BIGINT` / `INTEGER` | Audit: total injections |
| `created_at` | `TIMESTAMPTZ` / `TEXT` | |
| `updated_at` | `TIMESTAMPTZ` / `TEXT` | |
