---
title: "feat: UI Screenshot Documentation Pipeline"
type: feat
status: completed
date: 2026-03-04
origin: docs/brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md
---

# UI Screenshot Documentation Pipeline

## Overview

Build an on-demand screenshot pipeline for IronClaw's web gateway UI documentation. A single command (`./docs/scripts/capture-screenshots.sh`) produces named PNG screenshots saved to `docs/assets/screenshots/` **and generates or updates accompanying Mintlify documentation pages** that explain what each UI element does and how to use it.

The pipeline uses **Playwright test specs** (`docs/tests/`) so screenshots only capture verified, working UI states — not broken or partially-loaded pages. Running against a live IronClaw instance with seeded data ensures docs show what users actually see.

**Key capability:** Each screenshot is paired with auto-generated documentation that describes:
- What UI element is shown and its purpose
- How users interact with it (click, type, select, etc.)
- What outcomes to expect from interactions
- Related features or navigation options

---

## Problem Statement

The Mintlify documentation website (see [2026-03-03-feat-ironclaw-mintlify-documentation-website-plan.md](./2026-03-03-feat-ironclaw-mintlify-documentation-website-plan.md)) needs visual screenshots of the web gateway UI for user guides. Manual screenshot capture is:
- Time-consuming and inconsistent
- Prone to capturing broken or partially-loaded states
- Difficult to reproduce when UI changes
- Hard to maintain over time

**Additional challenge:** Screenshots alone don't explain how to use the UI. Documentation writers must manually write companion text describing what each element does, how to interact with it, and what outcomes to expect. This often leads to:
- Docs that show UI but don't explain functionality
- Inconsistent explanation quality across different sections
- Outdated text that doesn't match the current screenshot
- Missing context about related features or navigation

A programmatic screenshot pipeline that **also generates or updates explanatory documentation** ensures visual documentation stays current, accurate, and actually teaches users how to use the interface.

---

## Proposed Solution

A TypeScript/Node.js + Playwright screenshot system living in `docs/tests/` that:
1. Starts a local IronClaw instance (auto-building if needed)
2. Seeds realistic data via REST API
3. Captures screenshots of verified UI states (assertions pass first)
4. **Generates or updates Mintlify documentation pages** with embedded screenshots and explanatory content
5. Outputs screenshots to `docs/assets/screenshots/` and docs to `docs/ui-reference/`
6. Cleans up seed data and stops IronClaw

**Key decisions** (see brainstorm: [docs/brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md](../brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md)):
- Playwright tests verify UI before screenshot (no broken captures)
- TypeScript/Node.js (Playwright's native environment)
- Fully automated script handles IronClaw lifecycle
- Screenshots committed to git alongside docs that reference them
- **Documentation is auto-generated from test metadata** describing UI elements and interactions
- Configurable via `.env.screenshot` at repo root

---

## Technical Approach

### Architecture

```
ironclaw-src/
├── .env.screenshot              # Screenshot environment config (gitignored)
│
docs/
├── package.json                 # Adds @playwright/test devDependency
├── tests/
│   ├── playwright.config.ts     # Playwright config (baseURL, output dir, viewport)
│   ├── fixtures/
│   │   └── seed.ts              # REST API calls to seed data
│   ├── specs/
│   │   ├── web-chat.spec.ts     # Chat UI screenshots + doc metadata
│   │   ├── web-skills.spec.ts   # Skills tab screenshots + doc metadata
│   │   ├── web-routines.spec.ts # Routines tab screenshots + doc metadata
│   │   └── web-settings.spec.ts # Settings/extensions screenshots + doc metadata
│   └── templates/               # Doc generation templates
│       ├── ui-page.md.hbs       # Handlebars template for UI reference pages
│       └── partials/              # Reusable template components
├── scripts/
│   ├── capture-screenshots.sh   # Entry point: build, start, capture, generate docs
│   └── generate-docs.ts         # Doc generation script (reads test metadata, writes .mdx)
├── assets/
│   └── screenshots/               # Output directory (committed to git)
│       ├── web-chat-overview.png
│       ├── web-skills-tab.png
│       └── ...
└── ui-reference/                # Auto-generated or updated Mintlify pages
    ├── chat.mdx                 # Chat interface documentation
    ├── skills.mdx               # Skills tab documentation
    ├── routines.mdx             # Routines tab documentation
    └── settings.mdx             # Settings/extensions documentation
```

### Capture Flow (Fully Automated)

```
1. Developer runs: ./docs/scripts/capture-screenshots.sh
2. Script sources .env.screenshot from repo root
3. Script checks for target/release/ironclaw binary
   - If absent: cargo build --release
4. Script starts IronClaw in background with GATEWAY_ENABLED=true
5. Script polls health endpoint until ready
6. Playwright launches headless Chromium
7. fixtures/seed.ts calls REST API to create known state
8. Each spec navigates, asserts element visibility, captures screenshots
9. Screenshots saved to docs/assets/screenshots/
10. Seed data cleaned up via REST API
11. Script stops IronClaw background process
12. Playwright report shows pass/fail per scenario
```

### Key Configuration (playwright.config.ts)

```typescript
import { defineConfig, devices } from '@playwright/test';

function parseViewport(env: string | undefined) {
  if (!env) return null;
  const [w, h] = env.split('x').map(Number);
  return { width: w, height: h };
}

export default defineConfig({
  testDir: './specs',
  outputDir: '../assets/screenshots/',
  fullyParallel: false, // Sequential for consistent state
  workers: 1,
  use: {
    baseURL: process.env.IRONCLAW_URL ?? 'http://localhost:3001',
    viewport: parseViewport(process.env.SCREENSHOT_VIEWPORT) ?? { width: 1280, height: 800 },
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chromium'] },
    },
  ],
});
```

### Seed Data Strategy (fixtures/seed.ts)

```typescript
// REST API client to seed IronClaw with known state
const API_BASE = process.env.IRONCLAW_URL ?? 'http://localhost:3001';
const AUTH_TOKEN = process.env.IRONCLAW_TOKEN ?? 'screenshot-test-token';

export async function seedChatData() {
  // Send a message via REST API to create conversation history
  await fetch(`${API_BASE}/api/chat`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${AUTH_TOKEN}` },
    body: JSON.stringify({ message: 'Hello, this is a screenshot test message' }),
  });
}

export async function seedSkillData() {
  // Install a test skill via skills API
  await fetch(`${API_BASE}/api/skills/install`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${AUTH_TOKEN}` },
    body: JSON.stringify({ name: 'test-markdown-skill' }),
  });
}

export async function cleanupSeedData() {
  // Remove all seeded data for idempotency
  await fetch(`${API_BASE}/api/admin/cleanup-screenshot-data`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${AUTH_TOKEN}` },
  });
}
```

### Example Spec (web-chat.spec.ts)

```typescript
import { test, expect } from '@playwright/test';
import { seedChatData, cleanupSeedData } from '../fixtures/seed';

test.beforeAll(async () => {
  await seedChatData();
});

test.afterAll(async () => {
  await cleanupSeedData();
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
  await page.fill('[data-testid="chat-input"]', 'Tell me about IronClaw');
  await page.press('[data-testid="chat-input"]', 'Enter');

  // Capture while streaming
  await page.waitForSelector('[data-testid="streaming-indicator"]');
  await page.screenshot({
    path: '../assets/screenshots/web-chat-streaming.png',
  });
});
```

---

## Documentation Generation System

### Overview

Each screenshot spec includes **documentation metadata** that feeds a doc generator. This ensures every captured UI state has accompanying explanatory text describing what users see and how to interact with it.

### Doc Generation Flow

```
1. Playwright test executes and captures screenshot
2. Test exports metadata object describing:
   - UI elements visible in the screenshot
   - Purpose of each element
   - How to interact with them (click, type, select)
   - Expected outcomes from interactions
   - Related features or navigation
3. Doc generator (`docs/scripts/generate-docs.ts`) reads:
   - Screenshot file path
   - Metadata from spec
   - Template for the UI section
4. Generator produces/updates Mintlify `.mdx` file:
   - Embeds screenshot in <Frame> component
   - Writes descriptive text for each element
   - Includes usage instructions
   - Links to related documentation
5. Developer reviews generated docs before committing
```

### Documentation Metadata Format

Each spec exports a `docMetadata` object:

```typescript
// docs/tests/specs/web-chat.spec.ts
export const docMetadata = {
  page: 'chat',
  title: 'Chat Interface',
  description: 'The main chat interface for interacting with IronClaw',
  screenshots: [
    {
      filename: 'web-chat-overview.png',
      title: 'Chat Overview',
      elements: [
        {
          selector: '.message-list',
          name: 'Message History',
          description: 'Displays the conversation history between you and IronClaw',
          interaction: 'Scroll to view older messages',
        },
        {
          selector: '[data-testid="chat-input"]',
          name: 'Message Input',
          description: 'Type your messages or commands here',
          interaction: 'Click to focus, type your message, press Enter to send',
        },
        {
          selector: '[data-testid="streaming-indicator"]',
          name: 'Streaming Indicator',
          description: 'Shows when IronClaw is generating a response',
          interaction: 'Wait for the indicator to disappear before sending follow-up',
        },
      ],
      usage: 'Use the chat interface to ask questions, run commands, or have IronClaw perform tasks. Type your request and press Enter.',
      related: ['skills', 'routines'],
    },
    {
      filename: 'web-chat-streaming.png',
      title: 'Streaming Response',
      description: 'Shows a response being generated in real-time',
      elements: [
        {
          selector: '.streaming-message',
          name: 'Partial Response',
          description: 'The response appears word-by-word as it is generated',
          interaction: 'Read as it appears, or wait for completion',
        },
      ],
      usage: 'Longer responses stream in real-time. You can read as they appear or wait for the full response.',
      related: [],
    },
  ],
};
```

### Doc Generator (`docs/scripts/generate-docs.ts`)

```typescript
import { docMetadata as chatMetadata } from '../tests/specs/web-chat.spec';
import { docMetadata as skillsMetadata } from '../tests/specs/web-skills.spec';
// ... other imports

interface DocTemplate {
  frontmatter(metadata: DocMetadata): string;
  screenshotSection(screenshot: ScreenshotMetadata): string;
  elementDescription(element: ElementMetadata): string;
}

const mintlifyTemplate: DocTemplate = {
  frontmatter: (meta) => `---
title: '${meta.title}'
description: '${meta.description}'
---
`,

  screenshotSection: (screenshot) => `
## ${screenshot.title}

${screenshot.description}

<Frame>
  <img src="/assets/screenshots/${screenshot.filename}" alt="${screenshot.title}" />
</Frame>

### UI Elements

${screenshot.elements.map(el => `- **${el.name}**: ${el.description}`).join('\n')}

### How to Use

${screenshot.usage}
${screenshot.related.length > 0 ? `\n### Related Features\n\n${screenshot.related.map(r => `- [${r}](/ui-reference/${r})`).join('\n')}` : ''}
`,
};

function generateDoc(metadata: DocMetadata): string {
  let doc = mintlifyTemplate.frontmatter(metadata);

  for (const screenshot of metadata.screenshots) {
    doc += mintlifyTemplate.screenshotSection(screenshot);
  }

  return doc;
}

// Generate all docs
const allMetadata = [chatMetadata, skillsMetadata, routinesMetadata, settingsMetadata];

for (const meta of allMetadata) {
  const doc = generateDoc(meta);
  const outputPath = `docs/ui-reference/${meta.page}.mdx`;
  // Write to file (or update existing)
  console.log(`Generated ${outputPath}`);
}
```

### Generated Mintlify Output Example

**`docs/ui-reference/chat.mdx`:**

```markdown
---
title: 'Chat Interface'
description: 'The main chat interface for interacting with IronClaw'
---

## Chat Overview

The primary interface for communicating with IronClaw. View your conversation history, send new messages, and see responses stream in real-time.

<Frame>
  <img src="/assets/screenshots/web-chat-overview.png" alt="Chat Overview" />
</Frame>

### UI Elements

- **Message History**: Displays the conversation history between you and IronClaw
- **Message Input**: Type your messages or commands here
- **Streaming Indicator**: Shows when IronClaw is generating a response

### How to Use

Use the chat interface to ask questions, run commands, or have IronClaw perform tasks. Type your request and press Enter.

### Related Features

- [Skills](/ui-reference/skills)
- [Routines](/ui-reference/routines)

---

## Streaming Response

Shows a response being generated in real-time. Longer responses appear word-by-word as IronClaw thinks through the answer.

<Frame>
  <img src="/assets/screenshots/web-chat-streaming.png" alt="Streaming Response" />
</Frame>

### UI Elements

- **Partial Response**: The response appears word-by-word as it is generated

### How to Use

Longer responses stream in real-time. You can read as they appear or wait for the full response.
```

### Integration with Capture Script

The `capture-screenshots.sh` script runs doc generation after successful screenshot capture:

```bash
#!/bin/bash
set -e

# ... (setup and screenshot capture as before)

# Run Playwright tests
cd ../tests
npm install
npx playwright test

# Generate documentation from test metadata
echo "Generating documentation..."
npx ts-node ../scripts/generate-docs.ts

echo "Screenshots captured to docs/assets/screenshots/"
echo "Documentation generated in docs/ui-reference/"
echo ""
echo "Review the generated docs before committing:"
echo "  git diff docs/ui-reference/"
```

### Review Workflow

1. Developer runs `./docs/scripts/capture-screenshots.sh`
2. Screenshots are captured and saved
3. Documentation is generated/updated
4. Developer reviews changes with `git diff docs/`
5. Developer edits generated docs if needed (add edge cases, clarify instructions)
6. Developer commits both screenshots and documentation together

---

## System-Wide Impact

### Interaction Graph

The screenshot pipeline operates independently of production code:
- Docs build doesn't depend on screenshots (graceful fallback if missing)
- Screenshots are dev-generated, not CI-generated (avoids headless browser complexity in CI)
- No runtime dependencies on Playwright in the main application

### Error Propagation

| Failure Point | Handling |
|---------------|----------|
| IronClaw build fails | Script exits non-zero, screenshots not generated |
| IronClaw health check timeout | Script exits non-zero after 60s |
| Playwright assertion fails | Screenshot not written, Playwright report shows failure |
| Seed API call fails | Test fails, cleanup still runs |
| Screenshot write fails | Playwright error, script exits non-zero |

### State Lifecycle Risks

- Seed data must be cleaned up even if tests fail (afterAll hook)
- IronClaw process must be terminated even if tests fail (trap EXIT in script)
- Port conflicts: use high ports (e.g., 13001) or check availability

---

## Scope: Screenshots to Capture (Priority Order)

1. **Chat interface** — message thread, input, streaming response indicator
2. **Skills tab** — list of installed skills, search, install flow
3. **Routines tab** — cron and event routines listed
4. **Extensions tab** — installed extensions, auth status
5. **Settings** — provider config, connection status

---

## Acceptance Criteria

### Infrastructure
- [ ] `docs/package.json` includes `@playwright/test` devDependency
- [ ] `docs/tests/playwright.config.ts` configures output to `../assets/screenshots/`
- [ ] `docs/tests/fixtures/seed.ts` provides REST API helpers for seeding
- [ ] `docs/tests/specs/*.spec.ts` cover all Phase 1 UI features with `docMetadata` exports
- [ ] `docs/scripts/capture-screenshots.sh` entry point script exists and is executable
- [ ] `docs/scripts/generate-docs.ts` doc generator script exists
- [ ] `docs/ui-reference/` directory exists for generated documentation
- [ ] `.env.screenshot` template documented in contributing guide

### Screenshot Functionality
- [ ] Running `./docs/scripts/capture-screenshots.sh` produces PNG files in `docs/assets/screenshots/`
- [ ] Script auto-builds IronClaw if binary not found
- [ ] Script starts/stops IronClaw automatically
- [ ] Screenshots only saved when assertions pass
- [ ] Seed data is cleaned up after capture (idempotent)

### Documentation Generation
- [ ] Each Playwright spec exports `docMetadata` describing UI elements and usage
- [ ] Doc generator reads metadata and produces Mintlify `.mdx` files
- [ ] Generated docs include `<Frame>` components with embedded screenshots
- [ ] Each UI element has: name, description, and interaction instructions
- [ ] Generated docs include "How to Use" section with usage workflows
- [ ] Related features are linked in each generated page
- [ ] Generated docs are written to `docs/ui-reference/{feature}.mdx`

### Output Quality
- [ ] Screenshots are 1280x800 by default (configurable via `SCREENSHOT_VIEWPORT`)
- [ ] File naming follows pattern: `{feature}-{view}.png` (e.g., `web-chat-overview.png`)
- [ ] Images are committed to git and display correctly in Mintlify
- [ ] Generated documentation renders correctly in Mintlify
- [ ] Generated docs include proper frontmatter (title, description)

### Documentation
- [ ] `docs/CONTRIBUTING.md` includes screenshot update procedure
- [ ] `docs/CONTRIBUTING.md` includes doc generation review procedure
- [ ] Mintlify pages reference screenshots using `<Frame>` components
- [ ] UI reference pages explain what each element does and how to use it
- [ ] Troubleshooting section covers common capture failures

---

## Implementation Steps

### Task 1: Project Scaffolding ✓

**Status:** Completed - All scaffolding files created

### Task 2: Playwright Configuration ✓

**Status:** Completed - Config file created at `docs/tests/playwright.config.ts`

### Task 3: Seed Data Fixtures ✓

**Status:** Completed - Fixtures created at `docs/tests/fixtures/seed.ts`

### Task 4: Screenshot Specs ✓

**Status:** Completed - Four specs created with docMetadata exports

### Task 5: Documentation Generator ✓

**Status:** Completed - Generator script at `docs/scripts/generate-docs.ts`

### Task 6: Entry Point Script ✓

**Status:** Completed - Executable script at `docs/scripts/capture-screenshots.sh`

### Task 7: Documentation Updates ✓

**Status:** Completed - CONTRIBUTING.md updated with screenshot procedure

### Task 8: Environment Template ✓

**Status:** Completed - Template created at `.env.screenshot.template`

---

## Original Implementation Details

### Task 1: Project Scaffolding

**Files:**
- Create: `docs/package.json`
- Create: `docs/tests/playwright.config.ts`
- Create: `docs/tests/fixtures/seed.ts`
- Create: `docs/assets/screenshots/.gitkeep`

**`docs/package.json`:**
```json
{
  "name": "ironclaw-docs-screenshots",
  "private": true,
  "scripts": {
    "screenshots": "playwright test",
    "screenshots:update": "playwright test --update-snapshots"
  },
  "devDependencies": {
    "@playwright/test": "^1.42.0",
    "typescript": "^5.3.0"
  }
}
```

**`docs/tests/tsconfig.json`:**
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  }
}
```

### Task 2: Playwright Configuration

**`docs/tests/playwright.config.ts`:**
- Configure baseURL from `IRONCLAW_URL` env var
- Set viewport default to 1280x800
- Configure outputDir to `../assets/screenshots/`
- Single worker for sequential execution

### Task 3: Seed Data Fixtures

**`docs/tests/fixtures/seed.ts`:**
- REST API client for IronClaw
- Functions: `seedChatData()`, `seedSkillsData()`, `seedRoutinesData()`, `cleanupSeedData()`
- Use `IRONCLAW_TOKEN` for auth

### Task 4: Screenshot Specs

**`docs/tests/specs/web-chat.spec.ts`:**
- Test: Chat interface overview
- Test: Chat with streaming indicator

**`docs/tests/specs/web-skills.spec.ts`:**
- Test: Skills tab with installed skills
- Test: Skills search results

**`docs/tests/specs/web-routines.spec.ts`:**
- Test: Routines tab overview

**`docs/tests/specs/web-settings.spec.ts`:**
- Test: Settings/extensions view

### Task 5: Documentation Generator

**Files:**
- Create: `docs/scripts/generate-docs.ts`
- Create: `docs/scripts/tsconfig.json`
- Create: `docs/ui-reference/.gitkeep`

**`docs/scripts/generate-docs.ts`:**
```typescript
import { readFileSync, writeFileSync, mkdirSync } from 'fs';
import { dirname } from 'path';

interface ElementMetadata {
  selector: string;
  name: string;
  description: string;
  interaction: string;
}

interface ScreenshotMetadata {
  filename: string;
  title: string;
  description: string;
  elements: ElementMetadata[];
  usage: string;
  related: string[];
}

interface DocMetadata {
  page: string;
  title: string;
  description: string;
  screenshots: ScreenshotMetadata[];
}

// Import metadata from spec files (simplified - actual implementation would use dynamic imports)
const allMetadata: DocMetadata[] = [];

function generateFrontmatter(metadata: DocMetadata): string {
  return `---
title: '${metadata.title}'
description: '${metadata.description}'
---

`;
}

function generateScreenshotSection(screenshot: ScreenshotMetadata): string {
  const elementsList = screenshot.elements
    .map(el => `- **${el.name}**: ${el.description}`)
    .join('\n');

  const relatedSection = screenshot.related.length > 0
    ? `\n### Related Features\n\n${screenshot.related.map(r => `- [${r}](/ui-reference/${r})`).join('\n')}`
    : '';

  return `## ${screenshot.title}

${screenshot.description}

<Frame>
  <img src="/assets/screenshots/${screenshot.filename}" alt="${screenshot.title}" />
</Frame>

### UI Elements

${elementsList}

### How to Use

${screenshot.usage}${relatedSection}

---

`;
}

function generateDoc(metadata: DocMetadata): string {
  let doc = generateFrontmatter(metadata);

  for (const screenshot of metadata.screenshots) {
    doc += generateScreenshotSection(screenshot);
  }

  return doc;
}

// Main execution
async function main() {
  // Ensure output directory exists
  mkdirSync('../ui-reference', { recursive: true });

  // In real implementation, dynamically import from ../tests/specs/*.spec.ts
  // For now, placeholder for the actual metadata imports
  for (const meta of allMetadata) {
    const doc = generateDoc(meta);
    const outputPath = `../ui-reference/${meta.page}.mdx`;
    writeFileSync(outputPath, doc);
    console.log(`Generated: ${outputPath}`);
  }

  console.log('\nDocumentation generation complete!');
  console.log('Review the generated files in docs/ui-reference/ before committing.');
}

main().catch(console.error);
```

**`docs/scripts/package.json`:**
```json
{
  "name": "ironclaw-docs-generator",
  "private": true,
  "scripts": {
    "generate": "ts-node generate-docs.ts"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "ts-node": "^10.9.0",
    "typescript": "^5.3.0"
  }
}
```

### Task 6: Entry Point Script (Updated for Doc Generation)

**`docs/scripts/capture-screenshots.sh`:**
```bash
#!/bin/bash
set -e

# Source env config
if [ -f ../../.env.screenshot ]; then
  source ../../.env.screenshot
fi

IRONCLAW_BIN="${IRONCLAW_BIN:-../../target/release/ironclaw}"
SCREENSHOT_PORT="${SCREENSHOT_PORT:-13001}"

# Build if needed
if [ ! -f "$IRONCLAW_BIN" ]; then
  echo "Building IronClaw..."
  (cd ../.. && cargo build --release)
fi

# Start IronClaw
echo "Starting IronClaw on port $SCREENSHOT_PORT..."
GATEWAY_ENABLED=true \
GATEWAY_PORT=$SCREENSHOT_PORT \
GATEWAY_AUTH_TOKEN=screenshot-test-token \
CLI_ENABLED=false \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/tmp/ironclaw-screenshot.db \
  "$IRONCLAW_BIN" &
IRONCLAW_PID=$!

# Cleanup on exit
cleanup() {
  echo "Stopping IronClaw..."
  kill $IRONCLAW_PID 2>/dev/null || true
  rm -f /tmp/ironclaw-screenshot.db
}
trap cleanup EXIT

# Wait for health
for i in {1..60}; do
  if curl -s "http://localhost:$SCREENSHOT_PORT/api/health" > /dev/null; then
    echo "IronClaw ready"
    break
  fi
  sleep 1
done

# Run Playwright to capture screenshots
cd ../tests
npm install
npx playwright test

# Generate documentation from test metadata
echo "Generating documentation..."
cd ../scripts
npm install
npx ts-node generate-docs.ts

echo ""
echo "✓ Screenshots captured to docs/assets/screenshots/"
echo "✓ Documentation generated in docs/ui-reference/"
echo ""
echo "Review the changes before committing:"
echo "  git diff docs/"
```

### Task 7: Screenshot Specs with Metadata

**`docs/tests/specs/web-chat.spec.ts`:**
```typescript
import { test, expect } from '@playwright/test';
import { seedChatData, cleanupSeedData } from '../fixtures/seed';

// Documentation metadata exported for generator
export const docMetadata = {
  page: 'chat',
  title: 'Chat Interface',
  description: 'The main chat interface for interacting with IronClaw',
  screenshots: [
    {
      filename: 'web-chat-overview.png',
      title: 'Chat Overview',
      description: 'The primary interface for communicating with IronClaw. View your conversation history, send new messages, and see responses stream in real-time.',
      elements: [
        {
          selector: '.message-list',
          name: 'Message History',
          description: 'Displays the conversation history between you and IronClaw',
          interaction: 'Scroll to view older messages; click to select text',
        },
        {
          selector: '[data-testid="chat-input"]',
          name: 'Message Input',
          description: 'Type your messages or commands here',
          interaction: 'Click to focus, type your message, press Enter to send',
        },
        {
          selector: '[data-testid="streaming-indicator"]',
          name: 'Streaming Indicator',
          description: 'Shows when IronClaw is generating a response',
          interaction: 'Wait for the indicator to disappear before sending follow-up',
        },
      ],
      usage: 'Use the chat interface to ask questions, run commands, or have IronClaw perform tasks. Type your request and press Enter.',
      related: ['skills', 'routines'],
    },
  ],
};

test.beforeAll(async () => {
  await seedChatData();
});

test.afterAll(async () => {
  await cleanupSeedData();
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
```

**`docs/tests/specs/web-skills.spec.ts`:**
- Test: Skills tab with installed skills
- Export `docMetadata` with element descriptions and usage instructions

**`docs/tests/specs/web-routines.spec.ts`:**
- Test: Routines tab overview
- Export `docMetadata` with cron/event routine explanations

**`docs/tests/specs/web-settings.spec.ts`:**
- Test: Settings/extensions view
- Export `docMetadata` with provider config explanations

### Task 8: Documentation Updates

**`docs/CONTRIBUTING.md` additions:**
```markdown
## Updating Screenshots

When the web UI changes, regenerate screenshots:

```bash
./docs/scripts/capture-screenshots.sh
```

The script will:
1. Build IronClaw if needed
2. Start a temporary server
3. Capture verified screenshots
4. Clean up and exit

Commit the updated PNG files along with your docs changes.
```

### Task 7: Git Configuration

**Ensure screenshots are tracked:**
```bash
# Remove .gitkeep once real screenshots exist
git rm docs/assets/screenshots/.gitkeep
git add docs/assets/screenshots/*.png
```

---

## Dependencies & Risks

| Dependency | Impact | Mitigation |
|------------|--------|------------|
| IronClaw binary | Must build successfully | Script handles auto-build |
| Port availability | 13001 must be free | Configurable via env var |
| Playwright/Chromium | Large download | Only needed for docs dev |
| Headless browser | CI complexity | Run locally only (on-demand) |
| API stability | Seed fixtures may break | Update when API changes |

---

## Future Considerations

- **CI integration:** Could run in CI to detect visual regressions (separate from capture)
- **Annotation support:** `SCREENSHOT_ANNOTATE=true` flag for callout overlays using Sharp + Canvas
- **Dark mode screenshots:** Capture both light and dark themes
- **Mobile viewports:** Add mobile screenshots for responsive docs

---

## Sources & References

### Origin
- **Brainstorm document:** [docs/brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md](../brainstorms/2026-03-04-ui-screenshot-docs-brainstorm.md) — Key decisions carried forward: Playwright tests as capture mechanism, TypeScript/Node.js stack, auto-start IronClaw, commit screenshots to git

### Related Plans
- [2026-03-03-feat-ironclaw-mintlify-documentation-website-plan.md](./2026-03-03-feat-ironclaw-mintlify-documentation-website-plan.md) — Mintlify docs structure that consumes these screenshots
- [2026-02-24-e2e-infrastructure.md](./2026-02-24-e2e-infrastructure.md) — Separate Python E2E testing (different purpose, different stack)

### External References
- [Playwright Test Documentation](https://playwright.dev/docs/intro)
- [Mintlify Frame Component](https://mintlify.com/docs/components/frame)
