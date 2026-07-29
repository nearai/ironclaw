/**
 * Playwright e2e configuration.
 *
 * Two projects:
 *   storybook — component-level coverage of every @ironclaw/ui component,
 *               driven against the static Storybook build (specs in
 *               e2e/storybook). The webServer builds storybook-static and
 *               serves it with the zero-dep script in scripts/serve-static.ts.
 *   app       — full agent-workspace flows against the VITE_DEMO_MODE=1
 *               build (in-memory fixtures, no backend; specs in e2e/app),
 *               served by `vite preview`.
 *
 * Run with: corepack pnpm test:e2e   (or test:e2e:storybook / test:e2e:app)
 * Servers are reused when already running locally, so keeping
 * `corepack pnpm e2e:server:storybook` / `:app` up in a terminal skips the
 * rebuild between runs.
 */
import { defineConfig, devices } from "@playwright/test";

const STORYBOOK_PORT = 6106;
const APP_PORT = 5199;

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [["list"]],
  timeout: 30_000,
  expect: { timeout: 5_000 },
  use: {
    ...devices["Desktop Chrome"],
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "storybook",
      testDir: "e2e/storybook",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: `http://127.0.0.1:${STORYBOOK_PORT}`,
        // Storybook has no static-motion kill rule (unlike the app), so the
        // ~150ms interaction transitions run there. Forcing reduced motion
        // makes computed-style assertions instant and deterministic — and
        // exercises the tokens.css prefers-reduced-motion opt-out for real.
        contextOptions: { reducedMotion: "reduce" },
      },
    },
    {
      name: "app",
      testDir: "e2e/app",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: `http://127.0.0.1:${APP_PORT}`,
      },
    },
  ],
  webServer: [
    {
      command: "corepack pnpm run e2e:server:storybook",
      url: `http://127.0.0.1:${STORYBOOK_PORT}/iframe.html`,
      reuseExistingServer: !process.env.CI,
      timeout: 300_000,
    },
    {
      command: "corepack pnpm run e2e:server:app",
      url: `http://127.0.0.1:${APP_PORT}/`,
      reuseExistingServer: !process.env.CI,
      timeout: 300_000,
    },
  ],
});
