---
title: "Playwright Screenshot Pipeline with Token Authentication"
description: "Building a UI screenshot capture pipeline that auto-authenticates with IronClaw web gateway"
category: integration-issues
date: 2026-03-04
severity: medium
status: resolved
---

## Problem

Building an automated screenshot documentation pipeline for IronClaw's web gateway UI that:
1. Auto-detects running IronClaw instances
2. Captures screenshots of authenticated views (chat, skills, routines, settings, extensions, memory)
3. Passes authentication tokens correctly to bypass the login screen
4. Works with client-side routed tabs that return 404 when accessed directly

## Symptoms

- Screenshots were blank (22KB files showing only the login screen)
- Tests failed with "waiting for locator('#app') to be visible" timeout
- Direct navigation to `/routines`, `/skills`, etc. returned HTTP 404
- Environment variables from `.env.screenshot` weren't being passed to Playwright

## Root Cause

1. **Token URL Construction**: The `getIronClawUrlWithToken()` function was creating malformed URLs like `/?token=TOKEN/skills` when appending paths
2. **Client-Side Routing**: IronClaw's web gateway uses client-side routing; only `/` is served by the backend
3. **Environment Variable Loading**: The `pnpm screenshots` command wasn't loading `.env.screenshot` before running tests
4. **Authentication Flow**: The web UI requires token in URL → auto-authentication → app visibility; tests were timing out before auth completed

## Solution

### 1. Fixed URL Construction

Updated `docs/tests/fixtures/seed.ts`:

```typescript
export async function getIronClawUrlWithToken(path?: string): Promise<string> {
  const baseUrl = await getBaseUrl();
  const token = process.env.IRONCLAW_TOKEN ?? 'screenshot-test-token';

  // Build the URL: base + path (if provided) + ?token=
  let url = baseUrl;
  if (path) {
    const normalizedPath = path.startsWith('/') ? path : `/${path}`;
    url = baseUrl.endsWith('/') ? baseUrl.slice(0, -1) : baseUrl;
    url = `${url}${normalizedPath}`;
  }

  return `${url}?token=${token}`;
}
```

### 2. Updated Test Scripts to Load Environment

Modified `docs/package.json`:

```json
{
  "scripts": {
    "screenshots": "export $(grep -v '^#' .env.screenshot | xargs) && cd tests && pnpm exec playwright test"
  }
}
```

This loads `.env.screenshot` variables before running Playwright.

### 3. Client-Side Navigation Pattern

Instead of direct navigation to `/routines`, tests now:
1. Navigate to root with token: `/?token=TOKEN`
2. Wait for authentication: `await page.waitForSelector('#app', { state: 'visible' })`
3. Click tab buttons: `await page.click('button[data-tab="routines"]')`

Example from `docs/tests/specs/web-routines.spec.ts`:

```typescript
test('routines tab overview', async ({ page }) => {
  const ready = await isIronClawReady();
  if (!ready) {
    test.skip(true, 'IronClaw not running');
    return;
  }

  // Navigate to root with token
  const url = await getIronClawUrlWithToken('/');
  await page.goto(url);

  // Wait for auto-authentication
  await page.waitForSelector('#app', { state: 'visible', timeout: 10000 });
  await page.waitForTimeout(500);

  // Click the tab (client-side routing)
  await page.click('button[data-tab="routines"]');
  await page.waitForTimeout(500);

  // Capture screenshot
  await page.screenshot({
    path: '../assets/screenshots/web-routines-overview.png',
    fullPage: false,
  });
});
```

### 4. Auto-Detection of IronClaw Port

The `docs/tests/fixtures/seed.ts` includes port auto-detection:

```typescript
const CANDIDATE_PORTS = [3000, 3001, 3002, 3003, 3004, 3005,
                          3006, 3007, 3008, 3009, 3010, 8080, 13001];

async function checkPortHealth(port: number): Promise<boolean> {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
      method: 'GET',
      signal: AbortSignal.timeout(3000),
    });
    return response.status === 200;
  } catch {
    return false;
  }
}
```

## Files Changed

| File | Changes |
|------|---------|
| `docs/package.json` | Added env var loading to screenshots script |
| `docs/tests/fixtures/seed.ts` | Fixed `getIronClawUrlWithToken()` with optional path param |
| `docs/tests/specs/web-chat.spec.ts` | Updated to wait for auth before screenshot |
| `docs/tests/specs/web-routines.spec.ts` | Added tab click for client-side navigation |
| `docs/tests/specs/web-settings.spec.ts` | Added tab click for client-side navigation |
| `docs/tests/specs/web-skills.spec.ts` | Added tab click for client-side navigation |
| `docs/tests/specs/web-extensions.spec.ts` | New test for extensions view |
| `docs/tests/specs/web-memory.spec.ts` | New test for memory view |
| `docs/scripts/capture-screenshots.sh` | Added export statements for env vars |
| `docs/scripts/generate-docs.ts` | Added metadata for extensions and memory |

## Prevention

When building screenshot pipelines for SPAs:
1. Verify if routes are client-side (check if direct URL returns 404)
2. Load env vars explicitly in npm scripts
3. Wait for authentication before interacting with the app
4. Use tab/button clicks for client-side navigation, not `page.goto()`

## Test Commands

```bash
# Run screenshot tests
cd docs && pnpm screenshots

# Run specific test
cd docs/tests && pnpm exec playwright test specs/web-chat.spec.ts

# Run full pipeline
cd /home/opselite/ai_projects/ironclaw-src && bash docs/scripts/capture-screenshots.sh
```

## References

- IronClaw web gateway auth: `src/channels/web/static/app.js` lines 96-114
- Client-side routing: All tab routes (`/routines`, `/skills`, etc.) handled by JavaScript
- Related: `docs/.env.screenshot` configuration file
