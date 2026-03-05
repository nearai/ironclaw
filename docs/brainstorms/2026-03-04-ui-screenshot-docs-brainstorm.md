# UI Screenshot Documentation — Brainstorm

**Date:** 2026-03-04
**Status:** Ready for planning

---

## What We're Building

An on-demand screenshot pipeline for IronClaw's web gateway UI. Running a single command against a live IronClaw server produces named PNG screenshots, saved to `docs/assets/screenshots/`, which are then referenced inline in Mintlify documentation pages.

The pipeline uses **Playwright test specs** (`docs/tests/`) so screenshots only capture verified, working UI states — not broken or partially-loaded pages.

---

## Why This Approach

### Playwright tests as the capture mechanism

Playwright test specs that both verify UI behavior and produce screenshots. Benefits:
- Screenshots are only written when assertions pass — so a screenshot of the chat UI means the chat UI actually worked
- Tests are documented scenarios: each one maps directly to a docs section
- The same suite can be run in CI later to detect visual regressions
- Playwright is already available in the toolchain (Playwright MCP is installed)

### Real server, real data

Screenshots show a live IronClaw instance with seeded data (a conversation, a skill installed, a routine configured) rather than mocked state. This means the docs show what users actually see.

### Mintlify image embedding

Mintlify MDX pages reference screenshots as standard markdown images or `<Frame>` components:

```mdx
<Frame>
  <img src="/assets/screenshots/web-chat-overview.png" alt="Chat interface" />
</Frame>
```

---

## Key Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Tool | Playwright | Already in toolchain; test-first screenshot guarantee |
| Language | Node.js (JS/TS) | Playwright's native environment; matches docs node_modules setup |
| Test location | `docs/tests/` | Co-located with the docs they serve |
| Output location | `docs/assets/screenshots/` | Mintlify's expected public asset path |
| Entry point | `docs/scripts/capture-screenshots.sh` | Single command to run: sets env vars, calls `npx playwright test` |
| Server requirement | User provides a running IronClaw instance | Script accepts `IRONCLAW_URL` + `IRONCLAW_TOKEN` env vars |
| Screenshot naming | Feature-scoped slugs, e.g. `web-chat-overview.png`, `web-skills-tab.png` | Predictable; MDX pages can hardcode the filename |
| Seed data | Script seeds data via IronClaw's REST API before capturing | Ensures consistent, realistic content in screenshots |
| Seed cleanup | Clean up after capture | Idempotent; safe for shared dev DBs |
| Git strategy | Commit screenshots to git | Simple; versioned with the docs that reference them |
| Annotations | Plain captures by default; configurable | `SCREENSHOT_ANNOTATE=true` flag enables callout post-processing |
| Viewport | Configurable, default 1280x800 | Set via `SCREENSHOT_VIEWPORT=WxH` env var |
| IronClaw startup | Script starts the binary | Fully turnkey; no manual server start required |
| Binary | Build if not found | `cargo build --release` runs automatically if `target/release/ironclaw` is absent |
| IronClaw config | `.env.screenshot` at repo root | Dedicated env file for screenshot context; gitignored |

---

## Proposed Architecture

```
ironclaw-src/
  .env.screenshot              # IronClaw config for screenshot context (gitignored)

docs/
  package.json                 # Add @playwright/test as devDependency
  tests/
    playwright.config.ts       # Playwright config (baseURL, screenshot output dir, viewport)
    fixtures/
      seed.ts                  # API calls to create seed data (messages, skills, routines)
    specs/
      web-chat.spec.ts         # Chat UI screenshots + assertions
      web-skills.spec.ts       # Skills tab screenshots
      web-routines.spec.ts     # Routines tab screenshots
      web-settings.spec.ts     # Settings/extensions tab screenshots
  scripts/
    capture-screenshots.sh     # Entry point: builds binary if needed, starts IronClaw,
                               # runs playwright test, stops IronClaw
  assets/
    screenshots/               # Output directory — committed to git
      web-chat-overview.png
      web-skills-tab.png
      ...
```

### Capture flow (fully automated)

```
1. Developer runs: ./docs/scripts/capture-screenshots.sh
2. Script sources .env.screenshot from repo root
3. Script checks if ../../target/release/ironclaw exists
   - If not: runs `cargo build --release` from repo root
4. Script starts IronClaw binary in background (GATEWAY_ENABLED=true)
5. Script waits for health endpoint to respond (polls with retries)
6. Playwright starts headless Chromium
7. fixtures/seed.ts calls IronClaw REST API to set up known state
   (example: send a message, install a test skill, create a routine)
8. Each spec navigates, asserts element visibility, screenshots key states
9. Screenshots saved to docs/assets/screenshots/
10. Seed data cleaned up via REST API
11. Script stops IronClaw background process
12. Playwright report shows pass/fail for each scenario
```

### Key configuration (playwright.config.ts)

```typescript
export default defineConfig({
  use: {
    baseURL: process.env.IRONCLAW_URL ?? 'http://localhost:3001',
    viewport: parseViewport(process.env.SCREENSHOT_VIEWPORT) ?? { width: 1280, height: 800 },
    // Auth token injected as a header or cookie in a global setup fixture
  },
  outputDir: '../assets/screenshots/',
  // reuseExistingServer: true — script manages the process lifecycle externally
});
```

### Annotation flag

`SCREENSHOT_ANNOTATE=true` enables a post-processing step (using `sharp` + canvas) that adds callout overlays after plain screenshots are captured. Default is off — plain captures only.

### Example spec (sketch)

```typescript
// docs/tests/specs/web-chat.spec.ts
test('chat interface overview', async ({ page }) => {
  await page.goto('/');
  await page.waitForSelector('.message-list');
  await expect(page.locator('.chat-container')).toBeVisible();
  await page.screenshot({
    path: '../assets/screenshots/web-chat-overview.png',
    fullPage: false,
  });
});
```

---

## Scope: Which Features to Cover First

Priority order matches the Phase 1 docs content plan:

1. **Chat interface** — message thread, input, streaming response indicator
2. **Skills tab** — list of installed skills, search, install flow
3. **Routines tab** — cron and event routines listed
4. **Extensions tab** — installed extensions, auth status
5. **Settings** — provider config, connection status

---

## Open Questions

*None — all resolved.*

---

## Resolved Questions

| Question | Decision |
|---|---|
| Automation level | On-demand script (not CI) |
| Which UI | Web gateway (browser), not TUI |
| Real vs mocked data | Real data from a live server |
| Capture approach | Playwright tests + screenshots (not bare script) |
| Git strategy | Commit screenshots to git |
| Seed cleanup | Clean up after capture |
| Annotations | Configurable; default plain |
| Viewport | Configurable via env var; default 1280x800 |
| IronClaw startup | Script starts the binary automatically |
| Binary provisioning | Auto-build if not found (`cargo build --release`) |
| IronClaw config source | `.env.screenshot` at repo root (gitignored) |
