# Issue #1609: Admin Management Panel — Implementation Plan

## Overview

Add a web-based admin management panel for users, workspaces, roles, and usage
monitoring. The panel lives within the existing SPA as a new top-level "Admin"
tab (visible only to `admin` role users), with sub-tabs for each domain.

**Dependencies:**
- #1605 (User Management) — **DONE.** Full CRUD, suspend/activate, usage stats, secrets provisioning all exist in `src/channels/web/handlers/users.rs` and are wired in `server.rs`. DB trait methods (`list_users`, `get_user`, `create_user`, `delete_user`, `update_user_profile`, `update_user_role`, `update_user_status`, `user_usage_stats`, `user_summary_stats`) are all implemented.
- #1607 (Workspaces) — **NOT STARTED.** No workspace-as-tenant DB entity exists. The current `Workspace` type (`src/workspace/`) is a per-user memory filesystem, not a multi-tenant organizational unit. This feature needs a new `workspaces` DB table and API endpoints.
- #1608 (RBAC) — **PARTIAL.** Two roles exist: `admin` and `member`. The `AdminUser` extractor in `auth.rs` gates admin endpoints with `identity.role != "admin"` check. No granular permissions (e.g., `manage_users`, `view_usage`, `manage_workspaces`). RBAC expansion is optional for the initial admin panel — the binary admin/member model is sufficient for v1.

---

## 1. Route Structure

### Frontend Routes (SPA hash/tab navigation)

| Route | Description |
|-------|-------------|
| Admin tab > Users | User list, create, edit, suspend/activate, delete |
| Admin tab > Usage | Per-user LLM usage dashboard with period selector |
| Admin tab > Tokens | View/manage API tokens across all users |
| Admin tab > Secrets | Per-user secrets provisioning |
| Admin tab > Workspaces | Workspace list, create, assign users (blocked on #1607) |
| Admin tab > Roles | Role/permission management (blocked on #1608 expansion) |

**v1 scope:** Users, Usage, Tokens, Secrets sub-tabs. Workspaces and Roles are placeholder stubs with "Coming soon" messaging.

### Backend API Routes

All existing admin endpoints are already registered under `/api/admin/`. See section 2 for gap analysis.

---

## 2. API Endpoints — Existing vs. Needed

### Already Implemented (no backend changes needed)

| Method | Path | Handler | File |
|--------|------|---------|------|
| POST | `/api/admin/users` | `users_create_handler` | `handlers/users.rs` |
| GET | `/api/admin/users` | `users_list_handler` | `handlers/users.rs` |
| GET | `/api/admin/users/{id}` | `users_detail_handler` | `handlers/users.rs` |
| PATCH | `/api/admin/users/{id}` | `users_update_handler` | `handlers/users.rs` |
| DELETE | `/api/admin/users/{id}` | `users_delete_handler` | `handlers/users.rs` |
| POST | `/api/admin/users/{id}/suspend` | `users_suspend_handler` | `handlers/users.rs` |
| POST | `/api/admin/users/{id}/activate` | `users_activate_handler` | `handlers/users.rs` |
| GET | `/api/admin/usage` | `usage_stats_handler` | `handlers/users.rs` |
| GET | `/api/admin/users/{user_id}/secrets` | `secrets_list_handler` | `handlers/secrets.rs` |
| PUT | `/api/admin/users/{user_id}/secrets/{name}` | `secrets_put_handler` | `handlers/secrets.rs` |
| DELETE | `/api/admin/users/{user_id}/secrets/{name}` | `secrets_delete_handler` | `handlers/secrets.rs` |
| GET | `/api/profile` | `profile_get_handler` | `handlers/users.rs` |
| POST | `/api/tokens` | `tokens_create_handler` | `handlers/tokens.rs` |
| GET | `/api/tokens` | `tokens_list_handler` | `handlers/tokens.rs` |
| DELETE | `/api/tokens/{id}` | `tokens_revoke_handler` | `handlers/tokens.rs` |

### New Endpoints Needed

| Method | Path | Purpose | Blocked on |
|--------|------|---------|------------|
| GET | `/api/admin/tokens` | List ALL tokens across all users (admin view) | None — new handler |
| DELETE | `/api/admin/tokens/{id}` | Admin-revoke any token | None — new handler |
| GET | `/api/admin/usage/summary` | Aggregated usage dashboard data (totals, top users, cost trend) | None — new handler |
| GET | `/api/admin/workspaces` | List workspaces | #1607 |
| POST | `/api/admin/workspaces` | Create workspace | #1607 |
| GET | `/api/admin/workspaces/{id}` | Workspace detail | #1607 |
| PATCH | `/api/admin/workspaces/{id}` | Update workspace | #1607 |
| DELETE | `/api/admin/workspaces/{id}` | Delete workspace | #1607 |
| POST | `/api/admin/workspaces/{id}/members` | Add member to workspace | #1607 |
| DELETE | `/api/admin/workspaces/{id}/members/{user_id}` | Remove member | #1607 |

---

## 3. HTML/JS File Structure

### Approach: Extend Existing SPA

The current SPA pattern is:
- Single `index.html` with all markup (tab panels as `<div>` sections)
- Single `app.js` (6997 lines) with all logic
- Single `style.css`
- CDN libs (DOMPurify, Marked), Google Fonts, no build system
- `include_str!()` embeds files at compile time in `static_files.rs`
- Tab switching via `data-tab` buttons and `switchTab()` function
- Settings sub-tabs via `data-settings-subtab` and `switchSettingsSubtab()`

### Proposed Structure

Given the "no build system" constraint and the existing 7K-line `app.js`, the admin panel should be a **separate HTML page** served at `/admin` rather than adding 2000+ more lines to `app.js`. This matches the project files pattern (`/projects/{id}/`).

```
src/channels/web/static/
  index.html          # Existing SPA (add "Admin" link in header for admin users)
  app.js              # Existing (add admin link logic, ~10 lines)
  style.css           # Existing (add admin-specific styles, ~200 lines)
  admin/
    index.html        # Admin SPA shell
    admin.js          # Admin panel logic (~1500 lines est.)
    admin.css         # Admin-specific styles (~400 lines est.)
```

**Rationale for separate page:**
1. `app.js` is already 7K lines; adding 2K more degrades maintainability
2. Admin UI is rarely used (only by admins), no need to load it for all users
3. Separate compile-time `include_str!()` keeps binary size proportional to usage
4. Shared auth token is available via `localStorage` (same origin)

**Alternative considered (rejected):** Adding another top-level tab. This would require modifying `index.html` and `app.js` extensively and ship admin code to all users.

### File Details

**`admin/index.html`** — Admin SPA shell:
- Same `<head>` pattern: Google Fonts, DOMPurify, Marked from CDN
- Shared `style.css` from parent + admin-specific `admin.css`
- Sub-navigation sidebar (same pattern as settings sidebar)
- Content panels for: Users, Usage, Tokens, Secrets, Workspaces (placeholder), Roles (placeholder)
- Back-link to main SPA

**`admin/admin.js`** — Admin panel logic:
- Auth: read token from `localStorage['ironclaw-token']` (same key as main SPA)
- `apiFetch()` helper (copy from app.js or shared snippet)
- Sub-tab: Users — table with sort, filter, create/edit modal, suspend/activate/delete actions
- Sub-tab: Usage — period selector (day/week/month), per-user breakdown table, cost totals
- Sub-tab: Tokens — cross-user token list, revoke action
- Sub-tab: Secrets — user picker + secret list, create/delete
- Sub-tab: Workspaces — placeholder "Available after #1607"
- Sub-tab: Roles — placeholder "Available after #1608"

**`admin/admin.css`** — Admin styles:
- Dashboard card layout for usage summary
- Data table styles (sortable headers, pagination)
- Modal/dialog for user create/edit
- Status badges, cost formatting

---

## 4. Auth / Permission Checks

### Frontend

1. On main SPA load, fetch `/api/profile` (already done at line 175 of `app.js`)
2. If `profile.role === 'admin'`, show "Admin" link in header bar
3. On `/admin` page load, fetch `/api/profile` — if not admin, redirect to `/`
4. All API calls use the same `Bearer` token from `localStorage`

### Backend

1. All `/api/admin/*` endpoints already use the `AdminUser` extractor which returns 403 for non-admin users
2. New endpoints (`/api/admin/tokens`, `/api/admin/usage/summary`) must also use `AdminUser`
3. Static files at `/admin/*` are served without auth (the page itself is harmless; all data requires authed API calls)

### No changes needed to `auth.rs`

The existing `AdminUser` extractor and `AuthenticatedUser` extractor cover all cases.

---

## 5. Component Breakdown

### 5.1 Users Panel (v1)

**Already exists** as settings sub-tab (`settings-users` in `index.html`, lines 460-482). The admin panel version will be a superset:

| Component | Description | API |
|-----------|-------------|-----|
| User list table | Sortable columns: name, email, role, status, jobs, cost, last active, created | `GET /api/admin/users` |
| Create user form | Modal/inline form with display_name, email, role fields | `POST /api/admin/users` |
| User detail drawer | Slide-in panel showing full user info + metadata | `GET /api/admin/users/{id}` |
| Edit user | Inline editing of display_name, role, metadata | `PATCH /api/admin/users/{id}` |
| Suspend/Activate | Toggle buttons with confirmation dialog | `POST .../suspend`, `POST .../activate` |
| Delete user | Danger button with confirmation modal | `DELETE /api/admin/users/{id}` |
| Token banner | One-time token display after user creation | Inline after POST |

**Migration note:** The existing users sub-tab in settings can be replaced with a link to `/admin` or kept as a lightweight view.

### 5.2 Usage Dashboard (v1)

| Component | Description | API |
|-----------|-------------|-----|
| Period selector | Day / Week / Month toggle buttons | Query param to `GET /api/admin/usage` |
| Summary cards | Total calls, total tokens, total cost, active users | `GET /api/admin/usage/summary` (new) |
| Per-user breakdown | Table: user, model, calls, input/output tokens, cost | `GET /api/admin/usage` |
| User filter | Dropdown to filter usage to a single user | `?user_id=` param |
| Cost chart | Optional: simple bar chart via CDN lib (Chart.js) | Client-side from usage data |

### 5.3 Tokens Panel (v1)

| Component | Description | API |
|-----------|-------------|-----|
| All-tokens table | Shows tokens across all users with prefix, name, user, created, last_used, expires, status | `GET /api/admin/tokens` (new) |
| Revoke button | Admin can revoke any token | `DELETE /api/admin/tokens/{id}` (new) |
| Create-for-user | Button to create token for a specific user | `POST /api/tokens` with `user_id` |

### 5.4 Secrets Panel (v1)

| Component | Description | API |
|-----------|-------------|-----|
| User picker | Dropdown to select user | `GET /api/admin/users` |
| Secret list | Table: name, provider (values never shown) | `GET /api/admin/users/{id}/secrets` |
| Create/update secret | Form: name, value (password field), provider, expires | `PUT /api/admin/users/{id}/secrets/{name}` |
| Delete secret | Button with confirmation | `DELETE /api/admin/users/{id}/secrets/{name}` |

### 5.5 Workspaces Panel (placeholder, blocked on #1607)

- "Workspace management is available after the Workspaces feature (#1607) is implemented."
- Empty state with description of planned functionality

### 5.6 Roles Panel (placeholder, blocked on #1608 expansion)

- "Granular role management is available after RBAC expansion (#1608)."
- Current roles displayed as read-only: admin, member

---

## 6. File-by-File Change List

### New Files

| File | Size Est. | Description |
|------|-----------|-------------|
| `src/channels/web/static/admin/index.html` | ~200 lines | Admin SPA shell with sidebar nav and content panels |
| `src/channels/web/static/admin/admin.js` | ~1500 lines | Admin panel logic (all sub-tabs) |
| `src/channels/web/static/admin/admin.css` | ~400 lines | Admin-specific styles |
| `src/channels/web/handlers/admin.rs` | ~150 lines | New admin-only handlers: list all tokens, revoke any token, usage summary |

### Modified Files

| File | Changes |
|------|---------|
| `src/channels/web/handlers/mod.rs` | Add `pub mod admin;` |
| `src/channels/web/handlers/static_files.rs` | Add `admin_index_handler`, `admin_js_handler`, `admin_css_handler` serving `include_str!("../static/admin/...")` |
| `src/channels/web/server.rs` | Register new routes: `GET /admin` (static), `GET /admin/admin.js`, `GET /admin/admin.css`, `GET /api/admin/tokens`, `DELETE /api/admin/tokens/{id}`, `GET /api/admin/usage/summary` |
| `src/channels/web/static/index.html` | Add admin link element in header bar (hidden by default, shown via JS for admins) |
| `src/channels/web/static/app.js` | ~15 lines: show/hide admin link based on profile role check (already fetched at line 175) |
| `src/channels/web/static/style.css` | ~20 lines: admin link styling in header |
| `src/channels/web/CLAUDE.md` | Document new `/admin` routes and admin handler module |
| `src/db/mod.rs` | Add `list_all_tokens()` and `revoke_token_by_id()` to Database trait (admin variants that don't scope to a single user) |
| `src/db/postgres.rs` | Implement `list_all_tokens()`, `revoke_token_by_id()` |
| `src/db/libsql.rs` | Implement `list_all_tokens()`, `revoke_token_by_id()` |
| `docs/USER_MANAGEMENT_API.md` | Document new admin token and usage summary endpoints |

### Files NOT Changed

- `src/channels/web/auth.rs` — No changes needed; existing `AdminUser` extractor is sufficient
- `src/channels/web/types.rs` — New DTOs can go in the admin handler file or in types.rs
- `src/channels/web/sse.rs` — No SSE events needed for admin panel (polling-based)

---

## 7. Dependencies and Ordering

### Phase 1: Admin Panel Foundation (this issue, no blockers)

1. Create `admin/index.html`, `admin/admin.js`, `admin/admin.css`
2. Add static file handlers and routes for `/admin/*`
3. Add admin link to main SPA header
4. Implement Users sub-tab (migrate from settings sub-tab pattern)
5. Implement Usage sub-tab (uses existing `/api/admin/usage`)
6. Add `GET /api/admin/tokens` and `DELETE /api/admin/tokens/{id}` backend
7. Implement Tokens sub-tab
8. Implement Secrets sub-tab
9. Add Workspaces and Roles placeholder stubs

### Phase 2: After #1607 (Workspaces)

1. Add workspace DB schema and trait methods
2. Add workspace API endpoints
3. Fill in Workspaces sub-tab UI

### Phase 3: After #1608 Expansion (RBAC)

1. Expand roles beyond admin/member
2. Add permission assignments
3. Fill in Roles sub-tab UI
4. Update `AdminUser` extractor to check granular permissions

### Implementation Order (within Phase 1)

```
1. Static file serving for /admin/* (handlers/static_files.rs, server.rs)
2. admin/index.html shell + admin.css + minimal admin.js (auth check, sub-tab switching)
3. Users sub-tab (reuse existing APIs, no backend changes)
4. Usage sub-tab (reuse existing /api/admin/usage, add /api/admin/usage/summary)
5. DB: list_all_tokens, revoke_token_by_id (both backends)
6. handlers/admin.rs: token list + revoke endpoints
7. Tokens sub-tab in admin.js
8. Secrets sub-tab in admin.js
9. Admin link in main SPA
10. Placeholder stubs for Workspaces + Roles
```

---

## 8. Testing Strategy

**Rule compliance.** This plan follows `.claude/rules/testing.md` — specifically
"No mocks, prefer real implementations or stubs" — and the "Test Through the
Caller, Not Just the Helper" rule from the root `CLAUDE.md`. Every admin route
gates DB-backed side effects and authorization, so mocked handler tests are
explicitly forbidden here: they are surface-only coverage and have caused
regressions in this repo before. Primary coverage is **integration-level
through the real `/api/admin/*` HTTP routes and real DB methods**, exercised
against both persistence backends.

### 8.1 Primary Tier: Integration Tests Through Real `/api/admin/*` Routes

Run with `cargo test --features integration`. Tests live in
`tests/admin_panel_integration.rs` (new file) and drive the **actual axum
router** built by `src/channels/web/server.rs::build_router`, not a handler
called in isolation. The router is wired to a real `AppState` with a real
`Database` and the real middleware chain (including the `AdminUser` /
`AuthenticatedUser` extractors). HTTP requests are issued via `tower::ServiceExt`
(`oneshot` / `call`) or an ephemeral `hyper` server so the full middleware
stack runs.

Per `src/db/CLAUDE.md`, every persistence-touching admin route MUST be covered
against **both PostgreSQL and libSQL** backends. The harness parameterises over
`Database` implementations using the existing dual-backend test pattern (see
`tests/workspace_integration.rs` / `src/db/` integration tests) — one test
function per case, invoked once per backend.

#### 8.1.1 Auth-gating matrix (applied to EVERY `/api/admin/*` route)

For each of: `GET /api/admin/users`, `POST /api/admin/users`,
`GET /api/admin/users/{id}`, `PATCH /api/admin/users/{id}`,
`DELETE /api/admin/users/{id}`, `POST /api/admin/users/{id}/suspend`,
`POST /api/admin/users/{id}/activate`, `GET /api/admin/usage`,
`GET /api/admin/usage/summary`, `GET /api/admin/tokens`,
`DELETE /api/admin/tokens/{id}`, `GET /api/admin/users/{id}/secrets`,
`PUT /api/admin/users/{id}/secrets/{name}`,
`DELETE /api/admin/users/{id}/secrets/{name}`:

| Case | Setup | Expected |
|------|-------|----------|
| Unauthenticated | No `Authorization` header | 401, no DB mutation |
| Wrong scheme / garbage token | `Authorization: Bearer deadbeef` | 401, no DB mutation |
| Authenticated member (non-admin) | Real user + real token, `role = "member"` | 403, no DB mutation |
| Authenticated admin | Real user + real token, `role = "admin"` | 2xx, DB state matches |
| Revoked admin token | Admin token that was valid, then revoked via `DELETE /api/admin/tokens/{id}` in the same test | 401, no DB mutation |

Every case asserts **both** the HTTP status AND the resulting DB state via a
follow-up read through the real `Database` trait (e.g. `get_user`,
`list_tokens_for_user`, `user_usage_stats`) — never a handler-internal hook.

#### 8.1.2 Token revocation end-to-end (critical path)

Single test, both backends:

1. Seed two admin users `A1`, `A2`, each with a valid admin token `T1`, `T2`.
2. `GET /api/admin/tokens` with `T1` → 200, payload contains both `T1` and `T2`.
3. `DELETE /api/admin/tokens/{T2.id}` with `T1` → 200. Assert via direct
   `Database::get_token` that `T2` is marked revoked (or absent, per schema).
4. `GET /api/admin/users` with `T2` → 401. **This is the regression gate.**
   The assertion runs through the full middleware chain, so it catches bugs in
   the auth extractor cache, the revocation-check order, or the token lookup
   path — none of which a mocked handler test would see.
5. `GET /api/admin/users` with `T1` → 200 (sibling token still works).
6. Self-revoke variant: `DELETE /api/admin/tokens/{T1.id}` with `T1` → 200,
   then any follow-up call with `T1` → 401.

The revocation path uses the **real** `revoke_token_by_id` DB method — no
revocation store mock, no in-memory shim.

#### 8.1.3 Usage / reporting flows

Seed real users, real job records, and real usage rows via the `Database`
trait, then call `GET /api/admin/usage` and `GET /api/admin/usage/summary`
through the real route. Assertions:

- Response payload matches expected aggregation for the seeded data (per-user
  calls, tokens, cost; summary totals, top-N users).
- Period selector (`?period=day|week|month`, `?user_id=`) filters correctly
  against real DB rows.
- Non-admin caller → 403 and **zero** rows returned; the auth-gating matrix
  above is applied to both endpoints.
- Empty-DB case returns a well-formed zero payload, not 500.

#### 8.1.4 User CRUD, secrets, and token-for-user flows

Exercised through the real routes end-to-end, asserting both response shape
and `Database` state after each call:

- `POST /api/admin/users` → user row exists, initial token row exists.
- `POST /api/admin/users/{id}/suspend` → `status = "suspended"`; subsequent
  authentication as that user is rejected (drive this through a real authed
  request using the suspended user's token).
- `POST /api/admin/users/{id}/activate` → `status = "active"`; authentication
  works again.
- `DELETE /api/admin/users/{id}` → user row gone, owned tokens revoked, owned
  secrets gone. Verify via direct `Database` reads.
- `PUT /api/admin/users/{id}/secrets/{name}` and `DELETE .../secrets/{name}`
  → encrypted secret row present/absent in the real secrets store; value is
  never echoed back in any response body.

### 8.2 Secondary Tier: Unit Tests for Pure Helpers Only

Unit tests (`cargo test`, in `mod tests {}` at the bottom of each file) are
permitted ONLY for pure helpers with no I/O and no authorization effect —
for example, a usage-aggregation reducer over an in-memory `Vec<UsageRow>`, or
a period-parameter parser. **Unit coverage is never a substitute for the
integration tier on any route that gates DB side effects or authorization.**
Handler functions, DB methods, and anything that touches the `AdminUser`
extractor path MUST be covered at the integration tier above.

Explicitly out of scope at the unit tier (these would be surface-only
coverage and are forbidden by `.claude/rules/testing.md`):

- Mocked `Database` trait passed into an admin handler.
- Mocked `AdminUser` / `AuthenticatedUser` extractor.
- Mocked revocation store.
- Any test that calls an admin handler function directly without going
  through the axum router and middleware.

### 8.3 E2E Tests (`tests/e2e/`)

Browser-level smoke tests in Playwright/Python, on top of the integration
tier — not a substitute for it:

| Test | What |
|------|------|
| Admin panel access control | Login as member: `/admin` redirects or shows error. Login as admin: panel loads. |
| User CRUD flow | Create user in UI, verify in table, suspend, activate, delete. |
| Token management | Create token for user, verify in admin tokens list, revoke, verify revoked token cannot make API calls from a second browser context. |
| Usage dashboard | Trigger LLM calls, verify they appear in the dashboard with correct totals. |

### Manual Testing Checklist

- [ ] Admin link appears only for admin users in main SPA
- [ ] Admin link hidden for member users
- [ ] `/admin` page loads and authenticates from `localStorage` token
- [ ] Users sub-tab: create, list, edit, suspend, activate, delete
- [ ] Usage sub-tab: period selector works, data populates
- [ ] Tokens sub-tab: cross-user list, revoke
- [ ] Secrets sub-tab: user picker, list, create, delete
- [ ] Workspaces placeholder shows "coming soon"
- [ ] Roles placeholder shows current roles
- [ ] Mobile responsive (admin page)
- [ ] Dark/light theme works on admin page
- [ ] i18n keys added for admin panel strings

---

## 9. Open Questions

1. **Separate page vs. tab?** This plan recommends a separate `/admin` page. An alternative is a top-level tab hidden for non-admins (simpler routing, but bloats app.js for all users). Decision needed before implementation.

2. **Chart library for usage dashboard?** Options: (a) plain HTML tables only (simplest), (b) Chart.js from CDN (~70KB), (c) a lighter charting lib. The vanilla-JS constraint allows CDN libs.

3. **Pagination for user/token lists?** Current `GET /api/admin/users` returns all users. For deployments with 100+ users, server-side pagination (`?page=1&per_page=50`) should be added to the API. This can be deferred to a follow-up.

4. **Should the existing users settings sub-tab be removed or kept as a lightweight view?** Recommendation: keep it for now, add a "Full admin panel" link within it.
