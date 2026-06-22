# host

## 1.11.2

### Patch Changes

- 4761f96: Narrow static asset extension regex to prevent false positives on non-asset routes containing dots

## 1.11.1

### Patch Changes

- 94ad10d: Host now injects `trustedOrigins` from `CORS_ORIGIN` env var into the auth plugin's base variables, aligning Better Auth's CSRF/origin checks with the host's CORS policy. Explicit `auth.variables.trustedOrigins` in `bos.config.json` still takes precedence.

## 1.11.0

### Minor Changes

- 36b6cd7: Fix deployment workflows to publish updated `bos.config.json` with the `bos publish --deploy` path.

### Patch Changes

- 36b6cd7: Tighten CSP nonce handling across SSR, hydration, and fallback shells, and fix the BOS viewer bootstrap path.
- 36b6cd7: Restore public plugin RPC routing for the browser API contract and keep SSR/client hydration aligned under strict CSP.

## 1.10.0

### Minor Changes

- 2ccdb28: Ensure the deploy workflow checks out the triggering commit before publishing deployment URL updates.

## 1.9.3

### Patch Changes

- 3af34db: Version asset URLs to prevent stale-cache chunk failures

  Client boot assets (`remoteEntry.js`, `style.css`, plugin UI remote entries) now include a `?v=<integrity>` query parameter matching the SSR pattern. This ensures browsers and CDNs serve the correct asset set after each deploy, eliminating `ChunkLoadError` caused by cached `remoteEntry.js` referencing async chunks that no longer exist on the upstream deployment.

  Also fixes the `_viewer` regex from invalid `/^/+/` to `/^\/+/`.

## 1.9.2

### Patch Changes

- 684bcab: Fix production CSP, viewer, and sign-out bugs

  - **CSP nonce**: pass `cspNonce` to `ThemeProvider` so the `next-themes` inline bootstrap script satisfies `script-src 'nonce-...'`. Without this, the browser blocks the script, causing a React hydration mismatch (#418) and cascading failures.
  - **Viewer regex**: fix invalid regex `/^/+/` in `_viewer` HTML template to `/^\/+/` so `widgetPath` leading slashes are correctly stripped instead of causing a SyntaxError.
  - **Sign-out navigation**: add `router.invalidate()` before `navigate()` in both `UserNav` and `SecuritySettings` sign-out handlers. Without this, TanStack Router's `beforeLoad` auth guards read stale session state and redirect back to the login page instead of the home page.

## 1.9.1

### Patch Changes

- 8ef8f56: Only expose the API plugin's router publicly on `/api`

  Plugin routers (registry, projects, etc.) are no longer mounted as separate HTTP endpoints on `/api/<plugin>`. Only the API plugin contract is served, providing a single unified OpenAPI spec at `/api/spec.json` and Scalar docs at `/api`. Other plugins remain accessible internally via `pluginsClient` for server-to-server composition.

  **Breaking changes:**

  - `/api/<plugin>` routes (e.g. `/api/registry`, `/api/projects`) no longer serve plugin REST/RPC endpoints
  - `/api/rpc/<plugin>/<procedure>` paths no longer route to plugin RPC procedures
  - Individual plugin OpenAPI specs and docs pages are no longer available
  - When the API plugin is unavailable, all `/api/*` routes return 503 instead of per-plugin 503s

- 8ef8f56: Fix `createPluginsClient` to use Proxy composition instead of `Object.assign`, which silently dropped Proxy-resolved RPC methods from the API client
- 8ef8f56: Replace UI asset 302 redirects with reverse proxy to fix Cloudflare 403 errors

  The host now proxies all UI public assets (images, CSS, JS, fonts, favicons) through the host origin instead of 302-redirecting browsers to the Zephyr CDN. This eliminates cross-origin requests that Cloudflare blocks with 403 errors.

  **Breaking changes:**

  - `RenderOptions.assetsUrl` removed from `everything-dev/ui/types` — assets are now served from the host origin via root-relative paths
  - `RouterContext.assetsUrl` removed from `everything-dev/ui/types` — no longer needed since assets resolve through the host proxy
  - `getRemoteEntryScript()` removed from `everything-dev/ui/head` — use `getRemoteScripts()` which now returns `{ src: "/remoteEntry.js" }`
  - `RemoteScriptsOptions.assetsUrl` removed — `getRemoteScripts()` no longer needs an assets URL
  - `UnderConstruction` component: `assetsUrl` prop removed — images use rspack module imports directly
  - `ClientRuntimeConfig.assetsUrl` now set to the host origin (`requestUrl.origin`) instead of the CDN URL — existing consumers should note this value change

  **What changed:**

  - Host: `isUiPublicAssetPath()` deleted, logic inlined; `redirectUiAssetRequest()` replaced with `proxyUiAssetRequest()` using `proxyRequest()`
  - Host: `renderClientShell()` uses root-relative paths (`/favicon.ico`, `/remoteEntry.js`) instead of CDN URLs
  - Host: Plugin UI `<script>` tags use `/__mf/plugin-ui/${key}/remoteEntry.js` proxy paths
  - Host: `buildRuntimeClientConfig` sets `assetsUrl` to `requestUrl.origin`
  - UI: All `${assetsUrl}/path` references replaced with `/path` root-relative paths
  - UI: `new URL(importedAsset, assetsUrl)` pattern removed — rspack module imports used directly
  - UI: `/skill.md` fetched via root-relative path, no `assetsUrl` construction needed

## 1.9.0

### Minor Changes

- dea876c: Remove `cspNonce` from ClientRuntimeConfig, fix SSR asset URLs, dissolve style-chrome

  - **everything-dev**: Remove `cspNonce` from `ClientRuntimeConfigSchema` (was leaking server-only value to client). Add `cspNonce` to `RouterContext`. Remove from `CreateRouterOptions`.
  - **ui**: Fix SSR asset URL mismatch — server `assetPrefix` now uses `bosConfig.app.ui.production` CDN URL instead of `/`, so imported assets resolve to the same absolute URL on both SSR and client. Dissolve `style-chrome.tsx` into `_layout.tsx`. Remove all `useClientValue` calls for runtime config reads (now use `runtimeConfig` from route context directly). Move `cspNonce` from L1 prop into `RouterContext`. Remove `getCspNonce()` from auth client. Add `runtimeConfig` prop to `UnderConstruction`.
  - **host**: Stop merging `cspNonce` into `runtimeConfig` for client shell.

## 1.8.2

### Patch Changes

- d26ed95: Pass CSP nonce through SSR pipeline and redirect UI assets instead of proxying to fix Cloudflare Error 1000

  **CSP nonce passthrough (production CSP script/style blocking fix):**

  The host generated a CSP nonce per request but never forwarded it to TanStack Router's SSR renderer, causing all inline scripts and styles to be blocked by `script-src 'nonce-...' 'strict-dynamic'` in production.

  - **everything-dev/types**: Add `cspNonce?: string` to `CreateRouterOptions` and `RenderOptions` interfaces
  - **everything-dev/types**: Add `cspNonce` to `RenderOptionsWithApi` (inherited from `RenderOptions`)
  - **ui/router.server**: Forward `cspNonce` to TanStack Router as `ssr: { nonce }` in `createRouter` and `renderToStream`
  - **ui/\_\_root**: Apply `nonce` from `useRouter().options.ssr?.nonce` to the `<style>` tag for base styles
  - **host/program**: Remove `as any` cast from `renderToStream` call — `cspNonce` is now a typed property
  - **host/tests**: Add regression tests verifying nonce appears on `<script>` and `<style>` tags when `cspNonce` is provided

  **Cloudflare Error 1000 fix (static asset 403s):**

  When both the host (Railway behind Cloudflare) and UI deployment (Zephyr Cloud behind Cloudflare) are orange-clouded, server-to-server proxying triggers Cloudflare Error 1000 "DNS points to prohibited IP". Browser requests to Zephyr Cloud work fine; only the host's `fetch()` proxy was blocked.

  - **host/program**: Replace `proxyUiAssetRequest` (server-to-server `fetch` proxy) with `redirectUiAssetRequest` (HTTP 302 redirect). The browser follows the redirect directly to the Zephyr Cloud origin, bypassing the Cloudflare-to-Cloudflare proxy loop
  - **ui/style-chrome**: Prefix rspack-imported `built_on.png` and `built_on_rev.png` with `assetsUrl` from runtime config so images load directly from the UI deployment origin instead of through the host
  - **ui/skill**: Use `assetsUrl` instead of `hostUrl` to fetch `/skill.md` directly from the UI origin
  - **host/tests**: Update `ui-public-assets.test.ts` — all UI asset tests now verify 302 redirect behavior instead of proxied content

## 1.8.1

### Patch Changes

- 82db5c4: Require ssrIntegrity for tenant SSR — prevent no-cache-per-request MF instance creation

  Tenant SSR now requires both `ssrUrl` and `ssrIntegrity` to be present. Previously, a whitelisted tenant with `ssrUrl` but no `ssrIntegrity` would bypass the router module cache (`shouldCacheRouterModule` returns false without `ssrIntegrity`), causing a new Module Federation instance to be created on every SSR request — the same pattern that caused the production SSR failure.

  Also fixes pre-existing typecheck errors in host test files (Effect Either narrowing, FederationError type annotation).

## 1.8.0

### Minor Changes

- 1adfdee: Support account-relative tenant resolution on shared hosts so subdomains derive from the active runtime account instead of `label.near`, and allow nested tenant labels in the resolver and tests. Expose runtime lineage in the apps registry by deriving parent, root, depth, and extendsChain from `extends`, and add registry list filters for parent and root traversal.

### Patch Changes

- dd5a7d4: Fix production SSR by keeping the UI auth client local during server rendering and by resolving SSR-imported asset URLs from the UI remote instead of the host origin.
- 4629b80: Write host deployment URLs back to the root bos.config.json so release publishes can commit updated runtime URLs.

## 1.7.3

### Patch Changes

- b662086: Move the homepage BOS viewer into an isolated iframe surface backed by a host-rendered `/_viewer` page.

  - Update `ui/src/routes/_layout/index.tsx` to load the landing viewer through `/_viewer` while preserving `?path=` support.
  - Add a dedicated host-rendered `/_viewer` endpoint with scoped CSP framing rules so the viewer can run in production without weakening the rest of the app.
  - Bootstrap the NEAR BOS web component from the host page so the requested widget path is forwarded correctly into the viewer runtime.

## 1.7.2

### Patch Changes

- 6b72cfd: Add fixed-core tenant UI composition for shared hosts so subdomains can resolve BOS configs per request while keeping the host, auth, and API runtime stable. This also hardens tenant remote integrity verification with bounded streaming, background refresh for asset requests, and safer SSR cache invalidation for updated remotes.

## 1.7.1

### Patch Changes

- ef4a77b: Tighten the host CSP in production by switching to nonce-based script loading with `strict-dynamic` while keeping `unsafe-eval` for Module Federation. Also pass the host-provided CSP nonce into the NEAR auth client so wallet iframe scripts continue to run under the stricter policy.

## 1.7.0

### Minor Changes

- 521f85e: Fix SSR auth client injection, proxy test mock shape, and test config resolution

  - **host**: Pass `authClient` to SSR `renderToStream` so the host's pre-resolved auth client
    is reused instead of creating a new one from config. Export `toAuthClientContext` for use
    in program.ts. Proxy test mock updated to use correct `initialized.context` shape instead
    of putting handler directly on `initialized`.

  - **everything-dev**: Add optional `authClient` field to `RenderOptionsWithApi` type so
    callers can provide a pre-configured auth client for SSR rendering.

  - **ui**: `renderToStream` now uses `authClient` from render options when provided, falling
    back to `createAuthClient(runtimeConfig)` when not specified.

  - **host/tests**: Replace `process.env`-based `BOS_UI_URL`/`BOS_UI_SSR_URL` with production
    URL fallbacks from `bos.config.json` (`app.ui.production`, `app.ui.ssr`). Add
    `createMockAuthClient` helper returning a null-session auth client for SSR tests. Pass
    `session: null` and `authClient` in test render options to match production SSR semantics.

### Patch Changes

- 212ea6f: Clean up test infrastructure: proxy mock, dead env plumbing, and type cast

  - **host/tests**: Replace 80-line manual `AuthClient` mock with an 8-line
    `Proxy`-based mock that auto-implements any property, making it resilient
    to auth client API changes.
  - **host/tests**: Remove dead `vitest.setup.ts` and its `setupFiles` entry
    from `vitest.config.ts`. The `BOS_UI_URL`/`BOS_UI_SSR_URL` env var
    plumbing was unused after switching `loadTestRuntimeConfig` to read
    production URLs from `bos.config.json`. Simplify `global-setup.ts` to
    just build the UI dist (no HTTP server or env var setup needed).
  - **ui**: Remove unnecessary type cast in `renderToStream` —
    `renderOptions.authClient` is now typed directly via `RenderOptions`.
    Remove unused `AuthClient` type import.

## 1.6.1

### Patch Changes

- 52bb6cd: Fix spurious `serveStatic: root path './dist' is not found` warning in development by skipping the static file middleware when `./dist` doesn't exist.

## 1.6.0

### Minor Changes

- 81f2599: Add `title` and `description` fields to `bos.config.json`, runtime config, and `ClientRuntimeInfo`. SEO head metadata now reads `title`/`description` from `runtimeConfig.runtime` instead of hardcoded defaults. Also removes a debug console.log, fixes an outdated comment in app.ts, adds a Dockerfile comment, and adds a workflow comment for FCAK creation.

## 1.5.15

### Patch Changes

- ffa8200: Catalog-ify rspack/rsbuild packages and propagate via bos upgrade/sync

  - Add @rspack/core, @rspack/cli, @rsbuild/core, @rsbuild/plugin-react to root package.json catalog
  - Convert all workspace package.json rspack/rsbuild deps from version ranges to catalog: refs
  - Change every-plugin @rspack/core peerDep from exact 1.7.4 to range ^1.7.4
  - Add CATALOG_TOOL_PACKAGES to manifest-normalizer for catalog: conversion during init/sync
  - Extend bos upgrade to also bump catalog tool packages to latest npm versions
  - Extend bos status to report catalog tool package versions

- Updated dependencies [ffa8200]
  - every-plugin@2.5.9

## 1.5.14

### Patch Changes

- 6425196: Upgrade hono to >=4.12.18 to resolve 5 security vulnerabilities (CSS injection, JWT validation, cache leakage, XSS, bodyLimit bypass). Soften CI audit step to warn instead of fail on high/critical findings for build-time-only dependencies.

## 1.5.13

### Patch Changes

- cd7692f: Strengthen the generated auth surface and remove duplicate client facades so downstream packages rely on the canonical typed auth client.
- Updated dependencies [cd7692f]
  - every-plugin@2.5.8

## 1.5.12

### Patch Changes

- e2b4b85: Remove host/api/ui/plugins source from Docker image (loaded remotely at runtime). Remove deprecated `GATEWAY_DOMAIN` environment variable in favor of consistent `BOS_GATEWAY`.

## 1.5.11

### Patch Changes

- 6189953: Compile CLI to standalone binary in Dockerfile for faster cold starts. Remove deprecated `GATEWAY_DOMAIN` environment variable in favor of consistent `BOS_GATEWAY`.
- b193ad6: Fix `reqHeaders` runtime type to be a real `Headers` instance instead of `Record<string, string>`, preventing `TypeError: undefined is not a function` when calling `.get()` in plugin handlers
- Updated dependencies [b193ad6]
  - every-plugin@2.5.7

## 1.5.10

### Patch Changes

- 3a875e2: Fix OpenAPI spec blank page (CSP blocks CDN scripts), host assets 403, and add auth/plugin status to health endpoint

## 1.5.9

### Patch Changes

- 6822f5e: Fix OpenAPI spec page showing blank white screen, add auth/plugin status to health endpoint, and serve host assets locally instead of proxying to UI CDN

## 1.5.8

### Patch Changes

- ba974d4: Fix OpenAPI spec page showing blank white screen and add auth/plugin status to health endpoint
- 05c9fe2: Fix changeset CI errors: replace catalog: protocol for every-plugin dependency so changesets can resolve versions

## 1.5.7

### Patch Changes

- 13f68ff: Inject `getRawBody` and `reqHeaders` into oRPC handler context so plugins can verify webhook signatures

  - Host session middleware now clones the request body before oRPC consumes it, exposing `getRawBody()` in context for raw body access
  - Dev server middleware also injects `reqHeaders` and `getRawBody` (previously passed `context: {}`)
  - API, projects, registry, and template plugins declare `getRawBody` in their context schemas
  - API plugin `reqHeaders` type changed from `z.custom<Record<string, string>>()` to `z.record(z.string(), z.string())` for proper runtime validation

## 1.5.6

### Patch Changes

- 369c59b: Remove redundant auth plugin variables from `bos.config.json` and inject them at runtime instead.

  - **`host/src/services/plugins.ts`**: Added `baseVariables` parameter to `loadPluginEntry` so runtime-derived values can be merged before explicit `variables` from `bos.config.json`. When loading the auth plugin, the host now injects `account` (from `config.account`) and `domain` (from `config.domain`, defaulting to `"localhost:3000"` in development) as base variables. Explicit values in `bos.config.json` still take precedence if present.

  - **`bos.config.json`**: Removed the `app.auth.variables` block. `account`, `hostUrl`, and `uiUrl` are no longer required here since the host provides `account` and `domain` automatically at plugin initialization time.

- ddb9952: Extract auth plugin from monorepo and remove `BETTER_AUTH_URL` env dependency.

  - **Deleted `plugins/auth/`**: The auth plugin is now maintained as an external package and loaded at runtime via Module Federation. The `app.auth` entry in `bos.config.json` remains intact for runtime loading.

  - **`host/src/services/plugins.ts`**: Added `normalizeDomain(domain, env)` helper that:

    - Returns as-is if the domain already has `http://` or `https://`
    - Prepends `http://` for `localhost` / `127.0.0.1` in development
    - Prepends `https://` for everything else
    - Applied to `domain` and `hostUrl` base variables when loading the auth plugin.

  - **Removed `BETTER_AUTH_URL`**: Dropped from `.env.example` and `packages/everything-dev/src/plugin.ts` env generation. The auth plugin now derives its base URL from the normalized `hostUrl` variable passed by the host at initialization time.

## 1.5.5

### Patch Changes

- 543c595: Relaxed CORS origin check to allow any `https://` origin while still respecting `CORS_ORIGIN` for explicit allow-listing. Added `frameSrc` to the Content Security Policy to permit external `https:` frames, fixing blocked wallet iframe loads.

## 1.5.4

### Patch Changes

- a0c5784: Upgrade `@hono/node-server` to `^2.0.1` across host and everything-dev packages.

  Bump dev dependencies group:

  - `@biomejs/biome` `2.4.10` → `2.4.14`
  - `@effect/language-service` `^0.84.3` → `^0.85.1`
  - `@electric-sql/pglite` `^0.2.0` → `^0.4.5`
  - `@vitest/ui` `4.1.2` → `4.1.5`

## 1.5.3

### Patch Changes

- a38288d: Fix plugin error handling and shared dependency resolution in production.

  ### Host

  - Use `formatError()` instead of `error.message` when logging plugin initialization failures. Effect's `Data.TaggedError` has an empty `message` by default, so errors were appearing as `[Plugins] Error:` with no detail.
  - Mount a 503 stub router when the API plugin is unavailable, returning a proper JSON error body instead of an empty `{}` or 404.

  ### every-plugin

  - Re-throw non-ORPC errors from the `onError` interceptor so they propagate to the caller instead of being swallowed, which caused oRPC to serialize `undefined` as `{}`.

  ### Config

  - Move `better-auth` from `shared.plugins` to both `shared.ui` and `shared.plugins` in `bos.config.json` so it is shared correctly across both browser and server Module Federation boundaries.
  - Remove `drizzle-orm` from shared dependencies; it is an auth plugin implementation detail, not a runtime shared boundary.

## 1.5.2

### Patch Changes

- f185a6c: Remove `@opentelemetry/api` resolve.fallback stub.

  The package is now a direct dependency, so the `false` fallback workaround is no longer needed. Bundlers will resolve it normally.

## 1.5.1

### Patch Changes

- 516376e: Make Module Federation shared dependencies config-driven and fix Docker production runtime crash.

  **Problem:** `every-plugin` hardcoded `drizzle-orm` and `better-auth` as shared MF deps, but these are app-specific packages. In Docker's isolated linker mode, `import("drizzle-orm")` from `every-plugin` failed because the generic framework package does not declare them as dependencies.

  **Solution:**

  - **Core shared deps** (`every-plugin`, `effect`, `zod`, `@orpc/contract`, `@orpc/server`) remain hardcoded in `every-plugin` — these are what the framework itself needs.
  - **App-specific shared deps** moved to `bos.config.json` under `shared.plugins` (same shape as existing `shared.ui`).
  - `ModuleFederationService` now accepts runtime `appShared` config via Effect Context (`AppSharedDepsTag`) and dynamically imports configured packages with `import(name)`.
  - `PluginRuntimeConfig` gains optional `shared` field; `PluginService.Live` threads it through the layer chain.
  - `RuntimeConfigSchema` validates `shared.plugins` alongside `shared.ui`.

  **Build-time cleanup:**

  - Removed `better-auth`/`drizzle-orm` from `pluginSharedDependencies` in `packages/every-plugin/src/build/shared-deps.ts`.
  - Host `rsbuild.config.ts` now merges `bosConfig.shared.plugins` into build-time shared deps.

  **Production startup hardening:**

  - Added preflight validation in `bos start`: checks `shared.plugins` packages are resolvable, validates required secrets from auth/api/plugin configs, warns on missing values.
  - `CORS_ORIGIN` defaults to `https://<config.domain>` when unset in production.
  - Fixed empty error messages in plugin loading by adding `formatError()` helper that properly extracts Effect Cause chains.
  - Removed duplicate secret warnings from `secretsFromEnv` — consolidated in pre-startup validation.

  **Files changed:**

  - `packages/every-plugin/src/runtime/mf-config.ts`
  - `packages/every-plugin/src/runtime/services/module-federation.service.ts`
  - `packages/every-plugin/src/runtime/services/plugin.service.ts`
  - `packages/every-plugin/src/runtime/index.ts`
  - `packages/every-plugin/src/types.ts`
  - `packages/every-plugin/src/build/shared-deps.ts`
  - `packages/everything-dev/src/types.ts`
  - `packages/everything-dev/src/plugin.ts`
  - `host/src/services/plugins.ts`
  - `host/rsbuild.config.ts`
  - `bos.config.json`

## 1.5.0

### Minor Changes

- e53af6e: Add CSP with feature flag, integrity registry, on-chain attestation, and safe plugin client factory

  CSP: Add `CSP_STRICT` const (default false) that toggles between relaxed mode (`'unsafe-inline'` + `'unsafe-eval'`) and strict mode (nonce + `'strict-dynamic'`). Relaxed mode is the default because Module Federation requires `'unsafe-eval'`, making strict inline script enforcement moot. All other CSP directives (object-src, base-uri, frame-ancestors, connect-src, etc.) remain enforced regardless of mode. When strict mode is enabled, nonces are injected into HTML script tags and the runtime config.

  Integrity: Add `IntegrityRegistry` class for SRI hash tracking, `installIntegrityFetchHook` for MF lifecycle fetch interception, `verifyConfigAgainstChain` for on-chain attestation checks, and `startIntegrityMonitor` for periodic background re-verification.

  Safety: Wrap plugin client factories with `createSafeClientFactory` to prevent arbitrary context injection. Merge CSP headers into SSR responses.

### Patch Changes

- 0a67206: Refactor dev orchestrator to service-descriptor architecture; add NEAR auth contract routes (nonce, verify, profile, relay, view); consolidate session queries in UI; add source-map devtool for plugin builds
- 34207e4: Reorganize dev port assignments: host=3000, api=3001, auth=3002, ui=3003, ui-ssr=3004, plugins=3010+

  Fix dev TUI display: host always shows "running" with port, remote non-host services show "loaded" without port. Strip ANSI codes from log files, only tag stderr as [ERR] when content is actually error-like, and replace Effect.logInfo with console.log in host logger for clean output.

## 1.4.0

### Minor Changes

- ab0a308: Move auth from plugin to app-level infrastructure with oRPC contract generation

  Auth is now `app.auth` in bos.config.json instead of `plugins.auth`. The host loads the auth plugin as Phase 0 (app-level infrastructure) before other plugins. Session resolution and auth HTTP handler are provided through the auth plugin's oRPC client and initialized context, eliminating direct Better Auth coupling in the host. The `syncApiContractBridge` now generates typed auth contract clients in `api/src/plugins-client.gen.ts` and `ui/src/api-contract.gen.ts`, enabling plugins to call auth routes via `services.plugins.auth()` instead of importing the raw `Auth` type.

- 7c62044: Upgrade better-auth to 1.6.9, mature auth plugin, and add auth orchestration

  Auth plugin now uses Drizzle migrations with virtual:drizzle-migrations, Effect acquireRelease for DB lifecycle, and requires BETTER_AUTH_SECRET. Fixes API key and invitation method shapes for better-auth 1.6.9. The everything-dev CLI orchestrates auth as a first-class dev process. Host replaces Deferred with FiberHandle and resets federation state on shutdown.

- c0452e7: Renamed `productionIntegrity` to `integrity` across all schemas, build configs, and `bos.config.json`. Added `name` and `version` fields to `BosPluginRef`. Enhanced `bos plugin add` with `bos://account/plugins/name` registry resolution, manifest validation, and automatic integrity computation. Enhanced `bos plugin publish` with manifest validation, integrity computation, and FastKV plugin registry writes. Added generic KV routes (`kvGet`, `kvList`, `kvPrepareWrite`, `kvRelayWrite`) to the registry plugin.
- c29e058: Migrate auth from plugin to app-level infrastructure. Host mounts only the raw Better Auth handler; authClient is injected separately from pluginsClient. Plugins receive auth context per-request, not via injected clients. Projects plugin cleaned of auth-proxying routes. Deleted every-plugin/context.ts.

### Patch Changes

- 0dc8772: Fix host crash when accessing auth plugin initialized context
- 39588a1: Remove dead code: bootstrap script, drizzle/database infrastructure, and unused dependencies

  The host no longer has a local database — auth is handled by a runtime-loaded plugin. Removed bootstrap.ts (superseded by orchestrator's spawnRemoteHost), drizzle.config.ts (schema directory already deleted), DrizzleORMMigrations rspack plugin, $apiClient global declaration, and 11 unused dependencies (drizzle-orm, drizzle-kit, better-auth, better-near-auth, @libsql/client, @proj-airi/unplugin-drizzle-orm-migrations, @t3-oss/env-core, @fastnear/near-connect, web-vitals, @tanstack/react-query, @tanstack/react-router). Cleaned up Dockerfile and .env.example accordingly.

## 1.3.2

### Patch Changes

- 3627dd8: Fix production deploy EACCES errors: appuser now owns /app, /app/data, and .bos directories so runtime file creation (database.db, logs, pids) works correctly in the Docker container

## 1.3.1

### Patch Changes

- aeab5ce: Remove demo routes and fix plugin routing. API shell now only exposes `ping` and `authHealth` (with `requireAuth` middleware). Plugin-specific routes are registered before the base API catch-all in Hono, fixing 404s on `/api/rpc/{plugin}/*`. OpenAPI spec includes the current domain as an available server.

## 1.3.0

### Minor Changes

- b666191: Restructure Docker build and release pipeline

  - **Multi-stage Docker build** excludes `packages/` from the final image. The builder stage resolves `workspace:*` refs to npm versions (via `scripts/resolve-workspace-refs.ts`), installs from npm, then the final stage copies only app code + node_modules.
  - **Release pipeline** is now a single sequential job: npm publish gates Zephyr deploy and Docker build. If npm publish fails, nothing else runs.
  - **Start command** uses `bos start` (binary from npm) instead of `bun packages/everything-dev/cli.js`. Account and domain are read from `bos.config.json`.
  - **`everything-dev` and `every-plugin`** moved to `dependencies` in root `package.json` (runtime deps in Docker).
  - **`docker.yml`** is now `workflow_dispatch` only — the release workflow builds Docker inline.

## 1.2.0

### Minor Changes

- d4df05d: ## Infrastructure: CI optimization, Docker hardening, staging environments, config-driven architecture

  ### CI/CD improvements

  - **Consolidated lint + typecheck** into a single job (was 2 sequential), removing ~1-2 minutes per CI run
  - **Replaced `bun lint` + `bun format:check`** with single `biome ci .` command
  - **Pinned Bun version** to `"1.4"` in all workflows (was `latest`)
  - **Added native caching** via `setup-bun@v2` cache option (removed redundant `actions/cache`)
  - **Upgraded `actions/checkout`** from v6 to v4
  - **Parallelized typecheck** across packages using background processes (`& wait`)
  - **Staging deployment workflow** (`.github/workflows/staging.yml`) — builds `:staging` image on merge to main
  - **Preview deployment workflow** (`.github/workflows/preview.yml`) — builds `:pr-N` image per PR, comments preview URL
  - **CI workflows read domain from `bos.config.json`** via `jq` instead of hardcoding

  ### Docker hardening

  - **Non-root user**: Container now runs as `appuser` (UID 1001) instead of root
  - **Layer caching**: Dependencies installed before source code copy for better cache hits
  - **Bun 1.4**: Updated base image from `oven/bun:1.3.9-alpine` to `oven/bun:1.4-alpine`
  - **Added `curl` and `/health` healthcheck** with 30s interval
  - **Removed `Dockerfile.dev`**: Development flow uses `bos dev`, not a dev Docker image
  - **Added `railway.json`** for Railway deployment configuration with health checks

  ### Staging environment support

  - **Added `staging` field** to `BosConfigSchema` for staging domain configuration
  - **Added `--env` flag** to CLI start command supporting `production` and `staging` environments
  - **Updated `start` script** to accept `APP_ENV` environment variable for environment selection
  - **Staging mode** sets `GATEWAY_DOMAIN` from `config.staging.domain` and labels process as "Staging Mode"

  ### Config-driven architecture

  `bos.config.json` is now the single source of truth. All hardcoded values have been eliminated in favor of deriving from config at runtime or build time:

  - **Removed hardcoded defaults** from `package.json` start script — `--account` and `--domain` no longer have shell fallbacks; config is read from `bos.config.json`
  - **`BETTER_AUTH_URL`** now defaults to `config.hostUrl` instead of hardcoded `localhost:3000`
  - **`fastkv.ts`** mainnet fallback uses the actual `accountId` parameter instead of hardcoded `"dev.everything.near"`
  - **Host page title** uses `config.domain` instead of hardcoded `"everything.dev"`
  - **UI app name** is injected at build time from `bos.config.json` via rsbuild `source.define` (was hardcoded `"everything.dev"` in 15+ route files)
  - **UI `about.tsx`** registry query params use `activeRuntime.accountId`/`gatewayId` instead of hardcoded values

  ### Breaking changes

  - `BOS_ACCOUNT` and `GATEWAY_DOMAIN` are no longer default-encoded in Docker image — config comes from `bos.config.json`
  - Docker `CMD` no longer passes `--account` / `--domain` — use `APP_ENV` env var to switch environments
  - `BosConfigSchema` now includes optional `staging` field — existing configs are unaffected
  - `StartOptionsSchema` now includes optional `env` field — existing invocations are unaffected
  - UI `branding.ts` `APP_NAME` now reads from `import.meta.env.APP_NAME` with `"everything.dev"` fallback

### Patch Changes

- 96a492e: Fix Docker build for nested workspaces

  Replace broken `COPY */package.json ./*/` with `COPY . .` before `bun install`, so nested workspace directories (`plugins/*/`, `packages/*/`) are present when Bun resolves workspaces. Fixes preview PR Docker builds failing with "Workspace not found".

- 7e1286a: ## Security hardening: SRI integrity, CORS tightening, and config cleanup

  ### Subresource Integrity (SRI) for remote entries

  - **New `everything-dev/integrity` module** with `computeSriHash`, `computeSriHashForUrl`, and `verifySriForUrl` — single source of truth for all integrity operations
  - **Deploy hooks** now compute SHA-384 hashes of `remoteEntry.js` and write `productionIntegrity`/`ssrIntegrity` to `bos.config.json` on deploy
  - **Client-side SRI**: `<script>` tags for remote entries now include `integrity` and `crossorigin="anonymous"` attributes
  - **Server-side SRI verification** before loading SSR modules, API plugins, and UI federation remotes
  - **Integrity plumbing**: `productionIntegrity` and `ssrIntegrity` fields flow through `BosConfig` → `RuntimeConfig` → `ClientRuntimeConfig` → HTML rendering

  ### CORS hardening

  - **`host/src/services/auth.ts`**: Better Auth `trustedOrigins` now falls back to `[hostUrl, ...uiUrl]` instead of `[]` when `CORS_ORIGIN` is unset, aligning with Hono CORS middleware
  - **`host/src/program.ts`**: Production warning when `CORS_ORIGIN` is unset; fixed bug where empty `uiConfig.url` could be included as a CORS origin
  - **`packages/everything-dev/src/host.ts`**: CORS origins now include UI URL in fallback; production warning added
  - **Production warning** added for missing `BETTER_AUTH_SECRET`

  ### Config / type cleanup

  - **Removed `resolvedConfig` and `canonicalConfigUrl`** from `ClientRuntimeInfo` — these leaked arbitrary config data to the client
  - **Renamed `ActiveRuntimeInfo`** to `ClientRuntimeInfo` everywhere for consistency
  - **Deduplicated `SharedDepConfigSchema`** — now an alias for `SharedConfigSchema`
  - **Added `productionIntegrity`** to `BosConfigInput` interface, removing `as any` cast
  - **Added `testnet`** to `BosConfigSchema`

  ### Bug fixes

  - Fixed trailing slash inconsistency in host's SSR URL construction
  - Fixed SRI integrity check being inside Effect retry scope (now fails fast, only module loading is retried)
  - Added `integrity` verification to API plugin loading (`everything-dev/src/api.ts` and `host/src/services/plugins.ts`)

  ### Breaking changes

  - `ActiveRuntimeInfo` type removed — use `ClientRuntimeInfo`
  - `resolvedConfig` and `canonicalConfigUrl` removed from `ClientRuntimeInfo`
  - `BetterAuth` `trustedOrigins` default changed from `[]` to `[hostUrl, ...uiUrl]`

## 1.1.1

### Patch Changes

- 1cea1e1: Fix mixed content errors when behind reverse proxy (Railway, etc.)

  Added support for `X-Forwarded-Proto` and `X-Forwarded-Host` headers to correctly determine the request URL when the server is behind a reverse proxy. This fixes mixed content errors where HTTPS pages were making HTTP API requests.

  Also added `secureHeaders` middleware for additional security headers (X-Content-Type-Options, X-Frame-Options, etc.).

## 1.1.0

### Minor Changes

- 2c93dbb: Multi-tenant organization support with Better Auth integration

  - Added Better Auth organization plugin with teams support
  - Implemented all authentication methods: NEAR, email/password, phone OTP, passkey, anonymous
  - Personal organization auto-created for every non-anonymous user
  - Organization management UI: browse, create, switch, invite members
  - Real invitation flow with email notifications
  - Dev-preview email/SMS transport (logs to .dev-preview/ directory)
  - Account settings page for managing auth methods and security
  - Removed placeholder org RPCs - now using Better Auth directly
  - Added API key plugin support
  - Updated milestone-1 documentation

### Patch Changes

- 44393e7: Fix authentication flow in host program with proper session handling and proxy test coverage
- 44393e7: Add plugin support with improved module federation service, shared dependencies handling, and auth client integration
- 44393e7: Add security hardening with Dependabot configuration, SECURITY.md policy, and axios vulnerability mitigation
- 9cb973d: Abstract UI runtime into everything-dev package

  - Moved router creation, SSR rendering, and hydration into everything-dev/ui
  - Split package exports into ./ui/client (browser-safe) and ./ui/server (SSR)
  - Added networkId derivation from account suffix (testnet/mainnet)
  - Created canonical ui/src/app.ts barrel for apiClient, authClient, runtime helpers
  - Deleted ui/src/remote/\* indirection layer
  - Added API contract manifest with checksum for type sync
  - Added everything-dev types sync CLI command
