# ui

UI package for the app shell, routes, and client runtime.

## Runtime Surface

The package runtime lives in `everything-dev/ui`:

| Export | Purpose |
|--------|---------|
| `everything-dev/ui/client` | Browser router/runtime factory |
| `everything-dev/ui/server` | SSR router/runtime factory |
| `everything-dev/ui/types` | Shared router and head types |

The app-level barrel is `ui/src/app.ts` and is the preferred import for route code.

**Shared dependencies** (singleton via `bos.config.json → app.api.shared` and `app.auth.shared`):

- `react`, `react-dom`
- `@tanstack/react-query`, `@tanstack/react-router`
- `near-kit`
- `better-auth`, `better-near-auth`

## Development

```bash
bos dev                 # Typical: remote host (auto-detected), local UI + API
bos dev --api remote    # Isolate UI work
```

## Configuration

`bos.config.json` only needs the UI runtime URLs and package metadata. Build-time module exposes stay in `ui/rsbuild.config.ts`.

## Route Protection

File-based routing with auth guards via TanStack Router:

- `_authenticated.tsx` - Requires login, redirects to `/login`
- `_authenticated/_admin.tsx` - Requires admin role

## Tech Stack

- **Framework**: React 19
- **Routing**: TanStack Router (file-based)
- **Data**: TanStack Query + oRPC client
- **Styling**: Tailwind CSS v4 + shadcn/ui
- **Build**: Rsbuild + Module Federation
- **Auth**: better-auth client

## Scripts

- `bun dev` - Start dev server (port 3003)
- `bun build` - Build for production
- `bun type-check` - Type checking
