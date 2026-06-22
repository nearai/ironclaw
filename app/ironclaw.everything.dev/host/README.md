# host

Server host with authentication, Module Federation orchestration, and every-plugin runtime.

## Architecture

The host orchestrates both UI and API federation:

```
┌─────────────────────────────────────────────────────────┐
│                        host                             │
│                                                         │
│  ┌────────────────────────────────────────────────┐     │
│  │                  server.ts                     │     │
│  │  Hono.js + oRPC handlers                       │     │
│  └────────────────────────────────────────────────┘     │
│           ↑                         ↑                   │
│           │      bos.config.json    │                   │
│           │    (single source)      │                   │
│  ┌────────┴────────┐       ┌────────┴────────┐          │
│  │ UI Federation   │       │ API Plugins     │          │
│  │ (remoteEntry)   │       │ (every-plugin)  │          │
│  └────────┬────────┘       └────────┬────────┘          │
│           ↓                         ↓                   │
│  ┌─────────────────┐       ┌─────────────────┐          │
│  │ React app       │       │ oRPC router     │          │
│  │ (SSR/CSR)       │       │ (merged)        │          │
│  └─────────────────┘       └─────────────────┘          │
└─────────────────────────────────────────────────────────┘
```

Today the host boots from one base `RuntimeConfig` snapshot and keeps auth, API, and server-side plugin wiring fixed for the lifetime of the process.

On top of that fixed server core, the host now supports request-scoped tenant UI resolution:

- base config boots the host, auth, API, and plugin routers once
- subdomains resolve tenants by convention, for example `alice.linktree.com -> alice.near`
- tenant config must extend the base BOS runtime
- tenant requests may override UI-facing remotes and sidebar metadata without changing the server core

For full host/plugin/auth/api hot-swap, see [`../plans/runtime-config-hot-swap.md`](../plans/runtime-config-hot-swap.md). That is still a larger future design than the fixed-core tenant mode implemented now.

## Development

```bash
bos dev                 # Start development (host mode auto-detected)
bos dev                 # Full local development
```

## Production

```bash
bos start --no-interactive   # All remotes, production URLs
```

For the temporary publish registry, use `bos publish` or `bos publish --deploy`.

## Configuration

**bos.config.json**:

```json
{
  "app": {
    "host": {
      "title": "App Title",
      "description": "Description of the application",
      "development": "local:host",
      "production": "https://example.zephyrcloud.app",
      "secrets": [
        "HOST_DATABASE_URL",
        "HOST_DATABASE_AUTH_TOKEN",
        "BETTER_AUTH_SECRET",
        "BETTER_AUTH_URL"
      ],
      "template": "near-everything/every-plugin/demo/host",
      "files": [
        "rsbuild.config.ts",
        "tsconfig.json",
        "vitest.config.ts",
        "drizzle.config.ts"
      ],
      "sync": {
        "scripts": ["dev", "build", "test"]
      }
    }
  }
}
```

**Environment Variables:**

| Variable | Description | Default |
|----------|-------------|---------|
| `UI_SOURCE` | `local` or `remote` | Based on NODE_ENV |
| `API_SOURCE` | `local` or `remote` | Based on NODE_ENV |
| `API_PROXY` | Proxy API requests to another host URL | - |
| `NETWORK_ID` | Tenant account suffix resolution: `mainnet` or `testnet` | `mainnet` |
| `ALLOW_OVERRIDE` | Comma-separated tenant override targets like `ui`, `plugins.*`, `plugins.apps` | - |
| `TENANT_WHITELIST` | Comma-separated tenant account IDs allowed to SSR | - |
| `ALLOW_UNTRUSTED_SSR` | Allow tenant SSR without whitelist | `false` |
| `HOST_DATABASE_URL` | SQLite database URL for auth | `file:./database.db` |
| `HOST_DATABASE_AUTH_TOKEN` | Auth token for remote database | - |
| `BETTER_AUTH_SECRET` | Secret for session encryption | - |
| `BETTER_AUTH_URL` | Base URL for auth endpoints | - |
| `CORS_ORIGIN` | Comma-separated allowed origins | Host + UI URLs |

## Multi-Tenant Status

- Current: one process-wide base `RuntimeConfig`, fixed auth/API/plugin server core
- Current: tenant subdomains can resolve request-scoped UI remotes from FastKV-backed BOS config
- Current: tenant config must extend the base BOS runtime
- Current: tenant accounts derive relative to the active runtime account namespace
- Current: supported tenant overrides are `app.ui`, existing `plugins.<id>.ui`, and existing `plugins.<id>.sidebar`
- Current: tenant SSR is opt-in via `TENANT_WHITELIST` or `ALLOW_UNTRUSTED_SSR=true`
- Not yet implemented: tenant API/auth overrides in fixed-core mode
- Not yet implemented: dynamic new plugin IDs per tenant

## Tenant Mode

Example deployment:

```bash
BOS_ACCOUNT=linktree.near
BOS_GATEWAY=linktree.com
ALLOW_OVERRIDE=ui,plugins.*
TENANT_WHITELIST=alice.linktree.near,bob.linktree.near
ALLOW_UNTRUSTED_SSR=false
bos start --no-interactive
```

Example tenant behavior:

- `linktree.com` serves the base runtime
- `alice.linktree.com` resolves `bos://alice.linktree.near/linktree.com`
- `bob.linktree.com` resolves `bos://bob.linktree.near/linktree.com`
- nested labels compose too, such as `chicago.alice.linktree.com` -> `bos://chicago.alice.linktree.near/linktree.com`

Tenant config rules:

- must extend the base BOS runtime
- may only override targets allowed by `ALLOW_OVERRIDE`
- in fixed-core mode, only UI-facing overrides are applied
- custom UI remotes must provide integrity
- custom plugin UI remotes must provide integrity
- a child runtime with its own `account` and `domain` becomes a new tenant root on that domain even when it extends another runtime

Tenant SSR rules:

- if `ALLOW_UNTRUSTED_SSR=true`, any valid tenant UI with SSR config may SSR
- otherwise the tenant account must appear in `TENANT_WHITELIST`
- non-whitelisted tenants fall back to client rendering

### Proxy Mode

Set `API_PROXY=true` or `API_PROXY=<url>` to proxy all `/api/*` requests to another host:

```bash
API_PROXY=https://production.example.com bos dev
```

## Tech Stack

- **Server**: Hono.js + @hono/node-server
- **API**: oRPC (RPC + OpenAPI)
- **Auth**: Better-Auth + better-near-auth (SIWN)
- **Database**: SQLite (libsql) + Drizzle ORM
- **Build**: Rsbuild + Module Federation
- **Plugins**: every-plugin runtime

## Scripts

- `bun dev` - Start dev server (port 3000)
- `bun build` - Build MF bundle for production
- `bun bootstrap` - Run host from remote MF URL
- `bun preview` - Run production server locally
- `bun db:migrate` - Run migrations
- `bun db:studio` - Open Drizzle Studio

## Remote Host Mode

The host can be deployed as a Module Federation remote:

```bash
# Build and deploy
bos build host
bos deploy host

# Others can run from the remote URL
HOST_REMOTE_URL=https://your-zephyr-url.zephyrcloud.app bun bootstrap
```

## API Routes

| Route | Description |
|-------|-------------|
| `/health` | Health check |
| `/api/auth/*` | Authentication endpoints (Better-Auth) |
| `/api/rpc/*` | RPC endpoint (batching supported) |
| `/api/*` | REST API (OpenAPI spec at `/api`) |
