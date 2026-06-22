# Test Harness

## Shared Fake Reborn Server

Location: `tests/reborn-mock/`

This is a lightweight HTTP server that mimics the IronClaw Reborn WebChat v2 API. It supports:

- **Bearer token validation** on all endpoints
- **Mutable scenario state** per test (reset, setScenario)
- **Deterministic SSE streams** for chat event testing
- **9 predefined scenarios** covering healthy, failure, and streaming states

### Usage

```typescript
import { startRebornMock } from "../../../tests/reborn-mock";

const mock = await startRebornMock({ scenario: "healthy-chat" });
// mock.baseUrl -> "http://127.0.0.1:<ephemeral-port>"
// mock.token   -> "test-token-123"

// Use in API client:
const response = await fetch(`${mock.baseUrl}/api/webchat/v2/session`, {
  headers: { Authorization: `Bearer ${mock.token}` },
});

mock.setScenario("stream-gate");   // Switch scenario mid-test
mock.reset();                       // Reset to initial scenario
await mock.stop();                  // Cleanup
```

### Scenarios

| Name | Description |
|------|-------------|
| `healthy-empty` | Valid auth, no threads, no data |
| `healthy-chat` | Valid auth, 1 thread, automations, extensions, skills |
| `bad-token` | Token mismatch (tests auth rejection) |
| `unreachable` | Server refuses connection (no listener) |
| `stream-final-reply` | SSE: accepted → running → final_reply |
| `stream-gate` | SSE: accepted → gate prompt |
| `stream-failed` | SSE: accepted → failed with error |
| `stream-cancelled` | SSE: accepted → cancelled |
| `stream-drop-once` | SSE with reconnect scenario |

### Endpoints Implemented

- `GET /api/webchat/v2/session`
- `GET /api/webchat/v2/threads`
- `POST /api/webchat/v2/threads`
- `DELETE /api/webchat/v2/threads/:id`
- `POST /api/webchat/v2/threads/:id/messages`
- `GET /api/webchat/v2/threads/:id/timeline`
- `GET /api/webchat/v2/threads/:id/events?token=...`
- `GET /api/webchat/v2/automations`
- `GET /api/webchat/v2/outbound/preferences`
- `POST /api/webchat/v2/outbound/preferences`
- `GET /api/webchat/v2/outbound/targets`
- `GET /api/webchat/v2/extensions`
- `GET /api/webchat/v2/extensions/registry`
- `POST /api/webchat/v2/extensions/install`
- `GET /api/webchat/v2/skills`
- `POST /api/webchat/v2/skills/search`
- `POST /api/webchat/v2/skills/install`
- `GET /api/webchat/v2/channels/connectable`
- `GET /auth/providers`
- `POST /auth/session/exchange`
- `POST /auth/logout`

## Host + Mock Together

Location: `host/tests/helpers/reborn-app.ts`

Starts the full host (UI + API in-process) with the fake Reborn backend.

```typescript
import { startRebornApp } from "../helpers/reborn-app";
import { loginAnonymously } from "../helpers/playwright-auth";

const app = await startRebornApp();
// app.baseUrl        -> host URL
// app.rebornBaseUrl  -> mock Reborn URL
// app.rebornToken    -> mock bearer token

await loginAnonymously(page);
// Navigate to settings to seed the connection
await page.goto(`${app.baseUrl}/settings/ironclaw`);
await page.getByLabel("Tunnel URL").fill(app.rebornBaseUrl);
await page.getByLabel("API Token").fill(app.rebornToken);
await page.getByRole("button", { name: /save settings/i }).click();

await app.stop();
```

## Playwright Auth Helpers

Location: `host/tests/helpers/playwright-auth.ts`

Uses the real anonymous auth flow via better-auth.

```typescript
// Full login + navigate
await gotoAuthenticated(page, "/settings/ironclaw");

// Just login
await loginAnonymously(page);
```

## Running Tests

```bash
# E2E (Playwright)
bun run --cwd app/ironclaw.everything.dev/host test:e2e

# Integration (Vitest)
bun run --cwd app/ironclaw.everything.dev/host test

# Tagged integration tests
bun run --cwd app/ironclaw.everything.dev/host test -- --run
```
