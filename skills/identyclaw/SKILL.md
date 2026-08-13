---
name: identyclaw
version: "0.3.5"
description: >-
  IdentyClaw Passport sessions and federated peer login via builtin.idcp.
  Use for log in / login to IdentyClaw or discernible peer APIs (slcapi, etc.),
  HOLA, Passport, and apiEndpoint federation — never hand-roll HTTP login.
activation:
  keywords:
    - "identyclaw"
    - "hola"
    - "passport"
    - "rodit"
    - "peer agent"
    - "verify hola"
    - "api.identyclaw.com"
    - "idcp"
    - "federated"
    - "federated login"
    - "apiEndpoint"
    - "log in"
    - "login"
    - "slcapi"
    - "discernible"
  exclude_keywords:
    - "openclaw plugin install"
  patterns:
    - "(?i)\\bhola\\b"
    - "(?i)identyclaw"
    - "(?i)\\bidcp\\b"
    - "(?i)did:rodit:"
    - "(?i)passport\\s+id"
    - "(?i)verify.*(peer|agent|hola)"
    - "(?i)federat"
    - "(?i)apiEndpoint"
    - "(?i)ensure[_ ]session"
    - "(?i)log\\s*in\\s+to"
    - "(?i)\\blogin\\b.*https?://"
    - "(?i)https?://[^\\s]*discernible\\.io"
    - "(?i)https?://[^\\s]*slcapi"
    - "(?i)https?://[^\\s]*identyclaw"
  tags:
    - "identity"
    - "auth"
    - "interop"
  max_context_tokens: 1600
---

# IdentyClaw (IronClaw)

**Home (native IdentyClaw):** `https://api.identyclaw.com`  
**Federated peer:** any other Rodit-login HTTPS host (e.g. `https://slcapi.discernible.io:9443`)

These are **not** the same product surface. Federation shares **Rodit login only**
(timestamp → sign → JWT per host). A federated API **does not** need — and usually
**does not have** — the same endpoints as `api.identyclaw.com` (`/api/me/identity`,
HOLA, `/api/agents`, DID, …). Peer routes are whatever that product exposes.

## Login recipes (stop when done)

### Native / home

```json
{ "op": "ensure_session" }
```

Optional: `{ "op": "me" }` (omit `base`) — home Passport identity only.

### Federated peer (user named a non-home HTTPS URL)

```json
{ "op": "ensure_session", "base": "https://slcapi.discernible.io:9443" }
```

If `ok: true` and `federated: true` → **login succeeded. Reply and stop.**

Do **not** then call:
- `me` with that peer `base` (home-only; 404 is expected, not a failed login)
- `/api/health`, `/api`, root GET, OpenAPI probes, `builtin.http`, `result_read` loops
- HOLA / agents / DID against the peer

Only if the user asks for a **named product route** on that peer:

```json
{ "op": "request", "method": "GET", "path": "/their/path", "base": "https://slcapi.discernible.io:9443" }
```

## Home vs federated (do not assume shared routes)

| Concept | Home (`api.identyclaw.com`) | Federated peer |
|---------|----------------------------|----------------|
| How you log in | `ensure_session` (no `base`) | `ensure_session` + peer `base` |
| Success means | JWT for home | JWT for **that host only** |
| Endpoint set | IdentyClaw Passport/HOLA/DID | **Peer-specific — not a copy of home** |
| `me` / `/api/me/identity` | Yes | **Do not call** (404 ≠ login failure) |
| After login | optional `me` | **stop**, unless user named a product path |

Call **`builtin.idcp` only**. Never hand-roll `/api/login`. Never pass `http://127.0.0.1:3921` as `base`.

## `builtin.idcp` ops

| Op | Home | Federated |
|----|------|-----------|
| ensure_session | omit `base` | pass peer URL as `base` |
| me | omit `base` | **do not use** for login verification |
| request | omit `base` | same peer `base` + product path |
| create_hola / verify_hola / agents | home | do not assume peer has these |

## Rules

- Prefer **`builtin.idcp`** — never paste JWTs/keys.
- Federated `ensure_session` success **is** the login; do not “confirm” with home tools on the peer.
- If `ensure_session` fails, report the error and stop.
- After inbound `verify_hola` → `verified: true`, immediately `create_hola` on the same channel (home).
