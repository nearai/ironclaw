---
title: "Playwright Screenshot Pipeline for IronClaw Web UI"
description: "Auto-detecting screenshot capture pipeline with token-based authentication for documentation generation"
category: integration-issues
date: 2026-03-04
author: Claude Code
status: solved
components:
  - docs/tests/
  - docs/scripts/
  - docs/assets/screenshots/
symptoms:
  - Blank screenshots due to authentication failures
  - Environment variables not passed to Playwright tests
  - Client-side routes returning 404 when accessed directly
  - Malformed URLs with token in wrong position
root_causes:
  - pnpm scripts don't automatically load .env files
  - IronClaw web UI uses client-side routing (SPA)
  - Token must be passed in URL query parameter for auto-authentication
  - URL construction was appending paths after query parameters
---

## Problem

Build a screenshot capture pipeline for IronClaw documentation that:
1. Auto-detects running IronClaw instances
2. Captures screenshots of the web gateway UI (6 different views)
3. Passes authentication tokens correctly for automatic login
4. Generates Mintlify documentation from captured screenshots

### Symptoms Observed

- Screenshots were blank (22KB, indicating no content)
- Tests failed waiting for `#app` element to be visible (authentication never completed)
- Direct navigation to `/routines`, `/skills`, etc. returned 404
- URLs were malformed as `/?token=TOKEN/skills` instead of `/skills?token=TOKEN`

## Investigation Steps

### Step 1: Diagnose Authentication Flow

**Tried:** Check if token was being passed correctly
**Result:** Found that `.env.screenshot` wasn't being loaded by pnpm scripts
**Learning:** pnpm doesn't automatically source .env files like some other tools

```bash
# Tests passed when token was set explicitly:
IRONCLAW_TOKEN="..." pnpm exec playwright test
```

### Step 2: Fix URL Construction

**Tried:** Append paths directly to tokenized URLs
**Result:** Created malformed URLs: `/?token=TOKEN/settings`
**Solution:** Modify `getIronClawUrlWithToken()` to accept optional path parameter

```typescript
// Before: Malformed URL
`${baseUrl}${separator}?token=${token}/settings`

// After: Correct URL construction
const normalizedPath = path.startsWith('/') ? path : `/${path}`;
url = baseUrl.endsWith('/') ? baseUrl.slice(0, -1) : baseUrl;
return `${url}${normalizedPath}?token=${token}`;
```

### Step 3: Handle Client-Side Routing

**Tried:** Navigate directly to `/routines?token=TOKEN`
**Result:** 404 - these are client-side routes only
**Solution:** Navigate to root first, authenticate, then click tab buttons

```typescript
// Correct approach for SPA routes
await page.goto(await getIronClawUrlWithToken('/'));
await page.waitForSelector('#app', { state: 'visible' });
await page.click('button[data-tab="routines"]');
```

### Step 4: Fix Environment Variable Loading

**Tried:** Source .env.screenshot in capture script
**Result:** Variables available in script but not exported to child processes
**Solution:** Export variables explicitly and load in package.json script

```json
{
  "screenshots": "export $(grep -v '^#' .env.screenshot | xargs) && cd tests && pnpm exec playwright test"
}
```

## Working Solution

### 1. Environment Configuration (docs/.env.screenshot)

```bash
# Authentication token for API calls
IRONCLAW_TOKEN=your-token-here
IRONCLAW_URL=http://127.0.0.1:3000
```

### 2. Token Helper Function (docs/tests/fixtures/seed.ts)

```typescript
export async function getIronClawUrlWithToken(path?: string): Promise<string> {
  const baseUrl = await getBaseUrl();
  const token = process.env.IRONCLAW_TOKEN ?? 'screenshot-test-token';

  let url = baseUrl;
  if (path) {
    const normalizedPath = path.startsWith('/') ? path : `/${path}`;
    url = baseUrl.endsWith('/') ? baseUrl.slice(0, -1) : baseUrl;
    url = `${url}${normalizedPath}`;
  }

  return `${url}?token=${token}`;
}
```

### 3. Test Pattern for Client-Side Routes (docs/tests/specs/*.spec.ts)

```typescript
test('routines tab overview', async ({ page }) => {
  // Check if IronClaw is running
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

  // Click tab for client-side navigation
  await page.click('button[data-tab="routines"]');
  await page.waitForTimeout(500);

  // Capture screenshot
  await page.screenshot({
    path: '../assets/screenshots/web-routines-overview.png',
    fullPage: false,
  });
});
```

### 4. Auto-Detection Script (docs/scripts/capture-screenshots.sh)

```bash
# Source and export env config
if [ -f "$DOCS_DIR/.env.screenshot" ]; then
  echo "Loading configuration from docs/.env.screenshot..."
  source "$DOCS_DIR/.env.screenshot"
  # Export variables so they're available to child processes
  export SCREENSHOT_PORT
  export SCREENSHOT_HOST
  export IRONCLAW_URL
  export IRONCLAW_TOKEN
  export SCREENSHOT_VIEWPORT
  export HEALTH_TIMEOUT
fi

# Auto-detect IronClaw port
find_ironclaw_http_port() {
  for port in 3000 3001 3002 3003 3004 3005 3006 3007 3008 3009 3010 8080 13001; do
    response=$(curl -s -o /dev/null -w "%{http_code}" \
      "http://127.0.0.1:$port/api/health" 2>/dev/null || echo "000")
    if [ "$response" = "200" ]; then
      echo "$port"
      return 0
    fi
  done
  return 1
}
```

### 5. Package.json Scripts (docs/package.json)

```json
{
  "scripts": {
    "screenshots": "export $(grep -v '^#' .env.screenshot | xargs) && cd tests && pnpm exec playwright test",
    "screenshots:list": "export $(grep -v '^#' .env.screenshot | xargs) && cd tests && pnpm exec playwright test --list",
    "screenshots:update": "export $(grep -v '^#' .env.screenshot | xargs) && cd tests && pnpm exec playwright test --update-snapshots"
  }
}
```

## Key Insights

### IronClaw Web UI Authentication Flow

The IronClaw web UI (`src/channels/web/static/app.js`) has an `autoAuth()` function that:
1. Extracts token from URL query parameters (`?token=XXX`)
2. Sets the token in the input field
3. Calls `authenticate()` which tests the token against `/api/chat/threads`
4. On success: hides auth screen, shows app, initializes SSE connections
5. Cleans the token from URL (removes it from address bar)

This means:
- Token MUST be in query parameter format, not Authorization header
- Authentication is asynchronous (need to wait for `#app` to be visible)
- Session is stored in `sessionStorage` for subsequent navigation

### Client-Side vs Server-Side Routes

| Route | Type | Access Method |
|-------|------|---------------|
| `/` | Server | Direct navigation OK |
| `/routines` | Client-side | Navigate to `/` first, then click button |
| `/skills` | Client-side | Navigate to `/` first, then click button |
| `/memory` | Client-side | Navigate to `/` first, then click button |
| `/extensions` | Client-side | Navigate to `/` first, then click button |

## Prevention Strategies

1. **For SPA Screenshot Tests**: Always authenticate at root first, then use UI interactions for navigation
2. **Environment Variables**: Never assume shell exports propagate; explicitly export or use script loading
3. **URL Construction**: Always put query parameters at the end; use URL builder functions with optional path parameters
4. **Wait for Auth**: Always wait for authentication completion before assuming UI is ready

## Test Coverage

The pipeline now captures 6 views:
- Chat interface (`web-chat-overview.png`)
- Extensions tab (`web-extensions-overview.png`)
- Memory tab (`web-memory-overview.png`)
- Routines tab (`web-routines-overview.png`)
- Settings/logs tab (`web-settings-overview.png`)
- Skills tab (`web-skills-list.png`)

## Related Documentation

- [Mintlify Documentation](../../../ui-reference/)
- [Playwright Best Practices](https://playwright.dev/docs/best-practices)
- IronClaw web UI source: `src/channels/web/static/app.js` (autoAuth function)

## File Changes

```
docs/
├── .env.screenshot                    # Environment configuration
├── package.json                       # Updated scripts to load env vars
├── scripts/
│   ├── capture-screenshots.sh         # Auto-detection and orchestration
│   └── generate-docs.ts               # Metadata for extensions/memory added
├── tests/
│   ├── fixtures/seed.ts               # Fixed URL construction
│   └── specs/
│       ├── web-chat.spec.ts          # Updated with proper waits
│       ├── web-extensions.spec.ts    # NEW
│       ├── web-memory.spec.ts        # NEW
│       ├── web-routines.spec.ts      # Updated for client-side routing
│       ├── web-settings.spec.ts      # Updated for client-side routing
│       └── web-skills.spec.ts        # Updated for client-side routing
└── assets/screenshots/                # Generated screenshots
```
