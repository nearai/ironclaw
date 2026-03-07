---
title: "feat: UI Screenshot Documentation Pipeline"
type: feat
status: active
date: 2026-03-04
origin: docs/brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md
---

# UI Screenshot Documentation Pipeline

## Overview

Build an on-demand screenshot pipeline that captures IronClaw's web gateway UI for documentation. Running a single command (`./docs/scripts/capture-screenshots.sh`) against a live IronClaw server produces named PNG screenshots, saved to `docs/assets/screenshots/`, which are then referenced inline in Mintlify documentation pages.

The pipeline uses **Playwright test specs** (`docs/tests/`) so screenshots only capture verified, working UI states — not broken or partially-loaded pages.

**Key decisions (from brainstorm):**
- Playwright tests as capture mechanism — only screenshots verified UI states
- TypeScript/Node.js — Playwright's native environment; matches docs setup
- Real server with seeded data — shows actual user experience, not mocks
- Fully automated turnkey script — builds binary, starts server, captures, stops
- Committed to git — screenshots versioned with docs that reference them

---

## Problem Statement

The Mintlify documentation website needs accurate, up-to-date screenshots of the IronClaw web gateway UI. Manually capturing these is error-prone: screenshots become stale, UI elements may be missed, and developers must remember to re-capture after UI changes. Screenshots captured from broken or partially-loaded UIs mislead users.

---

## Proposed Solution

A TypeScript + Playwright test suite in `docs/tests/` that:
1. Builds IronClaw if needed
2. Starts the IronClaw server with mock LLM backend
3. Seeds realistic data via REST API (conversations, skills, routines)
4. Navigates to each UI section and captures screenshots after assertions pass
5. Cleans up seed data and stops the server
6. Outputs to `docs/assets/screenshots/` for Mintlify embedding

---

## Technical Approach

### Architecture

```
docs/
  tests/
    playwright.config.ts       # Playwright config (baseURL, output dir, viewport)
    fixtures/
      seed.ts                  # API calls to create seed data
      cleanup.ts               # API calls to clean up seed data
    specs/
      web-chat.spec.ts         # Chat UI screenshots
      web-skills.spec.ts       # Skills tab screenshots
      web-routines.spec.ts     # Routines tab screenshots
      web-extensions.spec.ts   # Extensions tab screenshots
      web-settings.spec.ts     # Settings screenshots
  scripts/
    capture-screenshots.sh     # Entry point: build, start, capture, cleanup
  assets/
    screenshots/               # Output directory — committed to git
      web-chat-overview.png
      web-chat-message-streaming.png
      web-skills-installed.png
      web-skills-search.png
      web-routines-list.png
      web-extensions-installed.png
      web-settings-provider.png
```

### Capture Flow (Fully Automated)

```
1. Developer runs: ./docs/scripts/capture-screenshots.sh
2. Script sources .env.screenshot from repo root
3. Script checks if ../../target/release/ironclaw exists
   - If not: runs cargo build --release from repo root
4. Script starts IronClaw binary in background with mock LLM
5. Script waits for health endpoint to respond (polls with retries)
6. Playwright starts headless Chromium
7. fixtures/seed.ts calls IronClaw REST API to set up known state
8. Each spec navigates, asserts element visibility, screenshots key states
9. Screenshots saved to docs/assets/screenshots/
10. fixtures/cleanup.ts removes seed data via REST API
11. Script stops IronClaw background process
12. Playwright report shows pass/fail for each scenario
```

### Mock LLM Server (Reuse E2E)

The Python mock LLM from `tests/e2e/mock_llm.py` will be reused. This avoids duplicating the mock LLM implementation and ensures consistency between E2E tests and screenshot capture.

**Alternative considered:** Separate TypeScript mock LLM. **Rejected** — adds maintenance burden, no benefit over Python version.

### Seed Data Strategy

Seed data creates realistic UI states:

| Spec | Seed Data |
|------|-----------|
| web-chat | Send 3 messages (user + assistant pairs), including one "What is 2+2?" |
| web-skills | Install "markdown" skill from ClawHub registry |
| web-routines | Create 2 routines: one cron, one event-driven |
| web-extensions | Install test extension with auth pending |
| web-settings | Configure mock LLM provider settings |

Seed data is created via IronClaw's REST API (`/api/jobs`, `/api/skills`, etc.) in `fixtures/seed.ts`.

### Configuration (playwright.config.ts)

```typescript
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './specs',
  outputDir: '../assets/screenshots/',
  fullyParallel: false, // Sequential to avoid state conflicts
  workers: 1,
  use: {
    baseURL: process.env.IRONCLAW_URL ?? 'http://localhost:3001',
    viewport: parseViewport(process.env.SCREENSHOT_VIEWPORT) ?? { width: 1280, height: 800 },
    screenshot: 'off', // Manual screenshots with custom filenames
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});

function parseViewport(env: string | undefined) {
  if (!env) return undefined;
  const [width, height] = env.split('x').map(Number);
  return { width, height };
}
```

### Entry Point Script (capture-screenshots.sh)

```bash
#!/bin/bash
set -e

# Source IronClaw config for screenshot context
if [ -f "../../.env.screenshot" ]; then
  source ../../.env.screenshot
fi

# Check for binary, build if missing
if [ ! -f "../../target/release/ironclaw" ]; then
  echo "Building IronClaw binary..."
  cd ../..
  cargo build --release
  cd - > /dev/null
fi

# Start mock LLM server (reuse from tests/e2e/)
MOCK_LLM_PORT=18199
python ../../tests/e2e/mock_llm.py --port $MOCK_LLM_PORT &
MOCK_LLM_PID=$!
echo "Mock LLM started on port $MOCK_LLM_PORT (PID: $MOCK_LLM_PID)"

# Start IronClaw with screenshot-specific config
export GATEWAY_ENABLED=true
export GATEWAY_HOST=127.0.0.1
export GATEWAY_PORT=18201
export GATEWAY_AUTH_TOKEN=screenshot-test-token
export GATEWAY_USER_ID=screenshot-tester
export CLI_ENABLED=false
export LLM_BACKEND=openai_compatible
export LLM_BASE_URL=http://127.0.0.1:$MOCK_LLM_PORT
export LLM_MODEL=mock-model
export DATABASE_BACKEND=libsql
export LIBSQL_PATH=:memory:
export SANDBOX_ENABLED=false
export SKILLS_ENABLED=true
export ROUTINES_ENABLED=true
export HEARTBEAT_ENABLED=false
export EMBEDDING_ENABLED=false
export ONBOARD_COMPLETED=true

../../target/release/ironclaw &
IRONCLAW_PID=$!
echo "IronClaw started (PID: $IRONCLAW_PID)"

# Wait for health endpoint
until curl -s http://127.0.0.1:$GATEWAY_PORT/api/health > /dev/null; do
  echo "Waiting for IronClaw to be ready..."
  sleep 1
done
echo "IronClaw is ready"

# Run Playwright tests
cd ..
npx playwright test tests/specs/ --reporter=line

# Cleanup
kill $IRONCLAW_PID 2>/dev/null || true
kill $MOCK_LLM_PID 2>/dev/null || true
echo "Screenshots captured to docs/assets/screenshots/"
```

### Example Spec (web-chat.spec.ts)

```typescript
import { test, expect } from '@playwright/test';
import { seedChatData, cleanupChatData } from '../fixtures/seed';

test.beforeAll(async () => {
  await seedChatData();
});

test.afterAll(async () => {
  await cleanupChatData();
});

test('chat interface overview', async ({ page }) => {
  await page.goto('/');
  await page.waitForSelector('.message-list');
  await expect(page.locator('.chat-container')).toBeVisible();

  await page.screenshot({
    path: '../assets/screenshots/web-chat-overview.png',
    fullPage: false,
  });
});

test('chat with streaming indicator', async ({ page }) => {
  await page.goto('/');

  // Send a message that triggers streaming
  await page.fill('#chat-input', 'What is 2+2?');
  await page.press('#chat-input', 'Enter');

  // Wait for streaming indicator to appear
  await page.waitForSelector('.streaming-indicator');

  await page.screenshot({
    path: '../assets/screenshots/web-chat-message-streaming.png',
    fullPage: false,
  });

  // Wait for response to complete for clean state
  await page.waitForSelector('.message.assistant', { timeout: 15000 });
});
```

---

## Scope: Features to Cover

Priority order matches Phase 1 docs content:

1. **Chat interface** — message thread, input, streaming response indicator
2. **Skills tab** — list of installed skills, search, install flow
3. **Routines tab** — cron and event routines listed
4. **Extensions tab** — installed extensions, auth status
5. **Settings** — provider config, connection status

---

## System-Wide Impact

### Interaction Graph

The screenshot pipeline is isolated from production code paths. Chain of execution:

```
capture-screenshots.sh
  → starts mock_llm.py (tests/e2e/)
  → starts ironclaw binary (release build)
  → npx playwright test
    → fixtures/seed.ts calls IronClaw REST API
    → specs/*.spec.ts navigate + screenshot
    → fixtures/cleanup.ts calls REST API cleanup
  → kills ironclaw process
  → kills mock_llm process
```

### Error Propagation

| Failure Point | Behavior |
|---------------|----------|
| Binary build fails | Script exits with cargo error |
| Health check fails | Script exits with a health-check error and does not run tests |
| Seed API call fails | Playwright test fails, seed cleanup still runs |
| Screenshot assertion fails | Test fails, no screenshot written, cleanup runs |
| Cleanup API call fails | Warning logged, script continues |

### State Lifecycle Risks
- **Seed data leak**: Cleanup runs in Playwright `test.afterAll` hooks to remove created seed data
- **Process leak**: Script is responsible for terminating all spawned processes on exit (including failure paths)
- **Port conflict**: Uses high ephemeral ports (18199, 18201) to avoid conflicts

---

## Acceptance Criteria

### Pipeline Infrastructure
- [ ] `docs/tests/playwright.config.ts` exists with proper config
- [ ] `docs/tests/package.json` includes `@playwright/test` devDependency
- [ ] `docs/scripts/capture-screenshots.sh` is executable and runs end-to-end
- [ ] Script builds binary if missing (`cargo build --release`)
- [ ] Script waits for health endpoint before running tests
- [ ] Script cleans up processes on exit (even on failure)

### Test Specs
- [ ] `docs/tests/specs/web-chat.spec.ts` captures chat overview screenshot
- [ ] `docs/tests/specs/web-skills.spec.ts` captures skills installed + search screenshots
- [ ] `docs/tests/specs/web-routines.spec.ts` captures routines list screenshot
- [ ] `docs/tests/specs/web-extensions.spec.ts` captures extensions screenshot
- [ ] `docs/tests/specs/web-settings.spec.ts` captures settings provider screenshot
- [ ] All specs include assertions before screenshot (verified UI state)
- [ ] All specs seed data before and cleanup after

### Output & Integration
- [ ] Screenshots saved to `docs/assets/screenshots/`
- [ ] Screenshots committed to git
- [ ] Screenshots referenced in Mintlify MDX pages via `/assets/screenshots/`
- [ ] All screenshots use consistent 1280x800 viewport
- [ ] Screenshots show realistic seeded data, not empty states

### Reproducibility
- [ ] Running script twice produces identical output (idempotent)
- [ ] Script works on clean clone (handles binary build)
- [ ] Script works on macOS and Linux

---

## Implementation Phases

### Phase 1: Core Pipeline (Day 1-2)

**Files:**
- `docs/tests/package.json` — Playwright dependency
- `docs/tests/playwright.config.ts` — Config
- `docs/tests/fixtures/seed.ts` — Seed data helpers
- `docs/tests/fixtures/cleanup.ts` — Cleanup helpers
- `docs/scripts/capture-screenshots.sh` — Entry point

**Steps:**
1. Create `docs/tests/package.json` with `@playwright/test`
2. Write `playwright.config.ts` with proper paths and viewport
3. Write shell script with build/start/capture/cleanup flow
4. Test script runs without error (no specs yet)

### Phase 2: Chat Screenshots (Day 2-3)

**Files:**
- `docs/tests/specs/web-chat.spec.ts`

**Screenshots:**
- `web-chat-overview.png` — Chat interface with message history
- `web-chat-message-streaming.png` — Streaming response indicator visible

**Steps:**
1. Write seed/cleanup for chat messages
2. Write spec with navigation and assertions
3. Verify screenshots look correct
4. Update Mintlify page to reference screenshots

### Phase 3: Skills & Routines (Day 3-4)

**Files:**
- `docs/tests/specs/web-skills.spec.ts`
- `docs/tests/specs/web-routines.spec.ts`

**Screenshots:**
- `web-skills-installed.png` — Installed skills list
- `web-skills-search.png` — Skills search results
- `web-routines-list.png` — Cron and event routines

**Steps:**
1. Write seed/cleanup for skills (install from registry)
2. Write spec for skills screenshots
3. Write seed/cleanup for routines
4. Write spec for routines screenshots

### Phase 4: Extensions & Settings (Day 4-5)

**Files:**
- `docs/tests/specs/web-extensions.spec.ts`
- `docs/tests/specs/web-settings.spec.ts`

**Screenshots:**
- `web-extensions-installed.png` — Extensions with auth status
- `web-settings-provider.png` — Provider configuration

**Steps:**
1. Write seed/cleanup for extensions
2. Write spec for extensions screenshots
3. Write seed/cleanup for settings
4. Write spec for settings screenshots

### Phase 5: Documentation Integration (Day 5-6)

**Files:**
- `docs/channels/web-gateway.mdx` — Add screenshots
- `docs/skills/*.mdx` — Add screenshots
- `docs/routines/*.mdx` — Add screenshots

**Steps:**
1. Review all screenshots for quality
2. Update MDX pages to reference screenshots
3. Test Mintlify dev server shows images correctly
4. Commit screenshots and documentation changes

---

## Alternative Approaches Considered

| Approach | Verdict |
|---|---|
| Reuse Python E2E tests for screenshots | Rejected — E2E tests are for validation, not documentation; Python Playwright screenshot API is different |
| Manual screenshot capture | Rejected — error-prone, screenshots become stale |
| Storybook or isolated components | Rejected — doesn't show real IronClaw UI with real data |
| CI-only screenshot generation | Rejected — developers need local preview for docs work |
| Docker-based capture | Rejected — adds complexity, native binary is simpler |

---

## Dependencies & Prerequisites

- Node.js 18+ (for Playwright)
- Rust toolchain (for building IronClaw)
- Python 3.11+ (for mock LLM server — reuse from tests/e2e/)
- Chromium (installed by Playwright)

---

## Resource Requirements

- **Time**: 5-6 days for full implementation
- **Storage**: ~5MB for screenshot assets
- **Compute**: Binary build takes ~5-10 minutes on modern hardware

---

## Documentation Plan

- `docs/tests/README.md` — How to run screenshot capture, add new specs
- `docs/CONTRIBUTING.md` — Update with screenshot maintenance workflow

---

## Sources & References

### Origin
- **Brainstorm document:** [docs/brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md](../brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md)
  - Key decisions carried forward: Playwright tests as capture mechanism, TypeScript/Node.js, real server with seeded data, fully automated turnkey script, committed to git

### Related Work
- E2E Infrastructure Plan: [docs/plans/2026-02-24-e2e-infrastructure.md](../plans/2026-02-24-e2e-infrastructure.md) — Reuse `mock_llm.py` and port configuration patterns
- Mintlify Docs Plan: [docs/plans/2026-03-03-feat-ironclaw-mintlify-documentation-website-plan.md](../plans/2026-03-03-feat-ironclaw-mintlify-documentation-website-plan.md) — Target documentation site structure
