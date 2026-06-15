<!-- intent-skills:start -->
# Skill mappings - load `use` with `npx @tanstack/intent@latest load <use>`.
skills:
  - when: "Install TanStack Devtools, pick framework adapter (React/Vue/Solid/Preact), register plugins via plugins prop, configure shell (position, hotkeys, theme, hideUntilHover, requireUrlFlag, eventBusConfig). TanStackDevtools component, defaultOpen, localStorage persistence."
    use: "@tanstack/devtools#devtools-app-setup"
  - when: "Publish plugin to npm and submit to TanStack Devtools Marketplace. PluginMetadata registry format, plugin-registry.ts, pluginImport (importName, type), requires (packageName, minVersion), framework tagging, multi-framework submissions, featured plugins."
    use: "@tanstack/devtools#devtools-marketplace"
  - when: "Build devtools panel components that display emitted event data. Listen via EventClient.on(), handle theme (light/dark), use @tanstack/devtools-ui components. Plugin registration (name, render, id, defaultOpen), lifecycle (mount, activate, destroy), max 3 active plugins. Two paths: Solid.js core with devtools-ui for multi-framework support, or framework-specific panels."
    use: "@tanstack/devtools#devtools-plugin-panel"
  - when: "Handle devtools in production vs development. removeDevtoolsOnBuild, devDependency vs regular dependency, conditional imports, NoOp plugin variants for tree-shaking, non-Vite production exclusion patterns."
    use: "@tanstack/devtools#devtools-production"
  - when: "Two-way event patterns between devtools panel and application. App-to-devtools observation, devtools-to-app commands, time-travel debugging with snapshots and revert. structuredClone for snapshot safety, distinct event suffixes for observation vs commands, serializable payloads only."
    use: "@tanstack/devtools-event-client#devtools-bidirectional"
  - when: "Create typed EventClient for a library. Define event maps with typed payloads, pluginId auto-prepend namespacing, emit()/on()/onAll()/onAllPluginEvents() API. Connection lifecycle (5 retries, 300ms), event queuing, enabled/disabled state, SSR fallbacks, singleton pattern. Unique pluginId requirement to avoid event collisions."
    use: "@tanstack/devtools-event-client#devtools-event-client"
  - when: "Analyze library codebase for critical architecture and debugging points, add strategic event emissions. Identify middleware boundaries, state transitions, lifecycle hooks. Consolidate events (1 not 15), debounce high-frequency updates, DRY shared payload fields, guard emit() for production. Transparent server/client event bridging."
    use: "@tanstack/devtools-event-client#devtools-instrumentation"
  - when: "TanStack Router bundler plugin for route generation and automatic code splitting. Supports Vite, Webpack, Rspack, and esbuild. Configures autoCodeSplitting, routesDirectory, target framework, and code split groupings."
    use: "@tanstack/router-plugin#router-plugin"
  - when: "Load environment variables from a .env file into process.env for Node.js applications. Use when configuring apps with secrets, setting up local development environments, managing API keys and database uRLs, parsing .env file contents, or populating environment variables programmatically. Always use this skill when the user mentions .env, even for simple tasks like \"set up dotenv\" — the skill contains critical gotchas (encrypted keys, variable expansion, command substitution) that prevent common production issues."
    use: "dotenv#dotenv"
  - when: "Use dotenvx to run commands with environment variables, manage multiple .env files, expand variables, and encrypt env files for safe commits and CI/CD."
    use: "dotenv#dotenvx"
  - when: "Build every-plugin modules with oRPC contracts, Effect services, and Module Federation. Use when creating or modifying plugins under plugins/ or the _template scaffold."
    use: "every-plugin#plugin-development"
  - when: "Test every-plugin modules with vitest and the plugin runtime. Use when writing or modifying plugin tests under plugins/*/src/__tests__/ or plugins/*/tests/."
    use: "every-plugin#plugin-testing"
  - when: "Development workflow for everything-dev projects using bos dev, bos start, and the Module Federation runtime. Use when starting dev servers, debugging hot reload, or understanding the service-descriptor architecture."
    use: "everything-dev#dev-workflow"
  - when: "How bos.config.json extends chains work, deep merge semantics, resolved config lifecycle, env-specific parents, tenant runtime inheritance, or debugging config merge behavior."
    use: "everything-dev#extends-config"
  - when: "Scaffold a new project, extend an existing project from a parent runtime, sync upstream files, upgrade framework packages, or choose local override sections for ui/api/host/plugins."
    use: "everything-dev#init-upgrade"
  - when: "Build a super app with a shared host and shared API, set up fixed-core tenant mode, reason about extends-based runtime lineage, configure tenant UI overrides, or create custom tenant apps that extend a base runtime."
    use: "everything-dev#super-app"
  - when: "Publish bos.config.json to the FastKV registry, sync from upstream, and upgrade workspace packages. Use when deploying, syncing, or managing runtime configuration across projects."
    use: "everything-dev#publish-sync"
  - when: "API architecture with oRPC contracts, route implementation, auth middleware patterns (requireAuth, requireRole, requireOrganization), error handling with ORPCError, plugin-client in-process composition, session middleware flow, and client-side auth integration."
    use: "everything-dev#api-and-auth"
  - when: "Build and register plugins within everything.dev using the _template scaffold, contract/service/index pattern, Drizzle database setup, bos.config.json registration, plugin UI and sidebar configuration, or deploying plugins to Zephyr CDN."
    use: "everything-dev#plugin-development"
  - when: "Create UI routes with TanStack Router, fetch data from API using useApiClient/useOrpc, implement auth flows with useAuthClient/sessionQueryOptions, configure the sidebar, understand SSR hydration, or use the @/app module surface."
    use: "everything-dev#ui-integration"
  - when: "Code style conventions — component file naming (kebab-case, lowercase), semantic Tailwind CSS (no hardcoded colors), no comments in implementation, import/export patterns, file/directory naming, and following neighboring file conventions."
    use: "everything-dev#code-style"
<!-- intent-skills:end -->

# Agent Instructions

This document provides operational guidance for AI agents working in this everything.dev project.

## Quick Reference

**Start Development:**
```bash
cp .env.example .env   # First time only
bun install
bun run dev
```

**Check Status:**
```bash
bos ps        # List running processes
bos status    # Project health check
bos info      # Show configuration
```

## Architecture

This is an everything.dev child project. Depending on your overrides, it may include:
- **UI** — React 19 + TanStack Router frontend, loaded via Module Federation
- **API** — Hono.js + oRPC backend with Effect services
- **Host** — Server-side runtime with Module Federation orchestration

The parent runtime provides the shared framework; your project provides custom overrides.

## Development Workflow

### Starting Development
1. `cp .env.example .env` (first time)
2. `bun install`
3. `bun run dev`

### Debugging Issues

**API not responding:**
- Check `bos ps` to see if the API process is running
- Check `.bos/logs/api.log` for errors

**UI not loading:**
- Verify the dev server is running: `bos ps`
- Check browser console for Module Federation errors
- Clear browser cache and retry

**Type errors:**
- Run `bun run typecheck`

## Code Changes

### Making Changes
- **UI Changes**: Edit `ui/src/` files → hot reload automatically
- **API Changes**: Edit `api/src/` files → hot reload automatically
- **Host Changes**: Edit `host/src/` when changing runtime resolution, auth wiring, SSR, proxying, or plugin mounting
- **New Components**: Create in `ui/src/components/ui/`, export from `ui/src/components/index.ts`
- **New Routes**: Create file in `ui/src/routes/`, TanStack Router auto-generates tree

### Style Requirements
- Use semantic Tailwind classes: `bg-background`, `text-foreground`, `text-muted-foreground`
- No hardcoded colors like `bg-blue-600`
- No code comments in implementation
- Component file naming: lowercase kebab-case (`data-table.tsx`, `user-profile.tsx`)
- Follow existing patterns in neighboring files

### Adding API Endpoints
1. Define in `api/src/contract.ts` — the oRPC route definitions and Zod schemas
2. Implement in `api/src/index.ts` — the `createRouter` function
3. Use in UI via `apiClient` from `useApiClient()` in `@/app`

## Testing & Quality

**Before committing:**
```bash
bun run test    # Run all tests
bun typecheck   # Type check all packages
bun lint        # Run linting
```

## Common Patterns

### Authentication Check

Routes requiring auth use `_authenticated.tsx` layout:
```typescript
export const Route = createFileRoute('/_layout/_authenticated')({
  beforeLoad: async ({ location }) => {
    const { data: session } = await authClient.getSession();
    if (!session?.user) {
      throw redirect({ to: '/login', search: { redirect: location.pathname } });
    }
  },
});
```

### API Client Usage
```typescript
import { useApiClient } from "@/app";

function MyComponent() {
  const apiClient = useApiClient();
  const { data } = await apiClient.ping();
}
```

## Troubleshooting

**Process won't start:**
```bash
bos kill        # Kill all tracked processes
bun install     # Ensure dependencies
bun run dev     # Restart
```

**Module Federation errors:**
- Check `bos.config.json` URLs are accessible
- Verify shared dependency versions match in package.json
- Clear browser cache

**Database issues:**
```bash
bun run db:push   # Push schema changes
bun run db:studio # Open Drizzle Studio
```

## Environment

**Required files:**
- `.env` — Secrets (see `.env.example`)
- `bos.config.json` — Runtime configuration (committed)
