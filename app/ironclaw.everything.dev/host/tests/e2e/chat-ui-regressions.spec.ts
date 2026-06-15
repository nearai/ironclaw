import { expect, test } from "@playwright/test";
import { startRebornMock } from "../../../tests/reborn-mock/server";
import { type BundledHostUrls, startBundledHost } from "../helpers/bundled-host";
import { loginAnonymously } from "../helpers/playwright-auth";

test.describe("Chat UI regressions", () => {
  test.describe.configure({ mode: "serial" });

  let app: { baseUrl: string; stop: () => Promise<void> };
  let rebornMock: {
    baseUrl: string;
    token: string;
    stop: () => Promise<void>;
    setScenario: (name: any) => void;
  };
  let savedSettings: { tunnelUrl: string; apiToken: string };
  let connected: boolean;

  test.beforeAll(async () => {
    rebornMock = await startRebornMock({ scenario: "healthy-empty" });
    savedSettings = { tunnelUrl: "", apiToken: "" };
    connected = false;

    app = await startBundledHost((urls: BundledHostUrls) => ({
      env: "production" as const,
      account: "test.near",
      networkId: "testnet" as const,
      title: "Test Host",
      repository: "https://github.com/test/repo",
      host: {
        name: "host",
        url: urls.baseUrl,
        entry: `${urls.hostAssetsUrl}/mf-manifest.json`,
        source: "local" as const,
      },
      ui: {
        name: "ui",
        url: urls.uiAssetsUrl,
        entry: `${urls.uiAssetsUrl}/mf-manifest.json`,
        source: "local" as const,
      },
      api: {
        name: "api",
        url: "",
        entry: "",
        source: "local" as const,
      },
    }));
  });

  test.beforeEach(async ({ page }) => {
    // Reset test state before each test
    savedSettings = { tunnelUrl: "", apiToken: "" };
    connected = false;

    // Register ironclaw API mocks with shared mutable state
    await page.route("**/api/rpc/ironclaw/**", async (route) => {
      try {
        const reqUrl = route.request().url();
        let body: any = {};
        try {
          body = route.request().postDataJSON() ?? {};
        } catch {}
        const match = reqUrl.match(/\/api\/rpc\/ironclaw\/(.+)/);
        const p = (match ? match[1] : (body?.procedure ?? "")).replace(/\//g, ".");

        if (p.includes("settings.get")) {
          if (!savedSettings.tunnelUrl) {
            await route.fulfill({ status: 404, body: JSON.stringify({ error: "NOT_FOUND" }) });
            return;
          }
          await route.fulfill({
            status: 200,
            contentType: "application/json",
            body: JSON.stringify(savedSettings),
          });
          return;
        }
        if (p.includes("settings.update")) {
          savedSettings = { tunnelUrl: rebornMock.baseUrl, apiToken: rebornMock.token };
          connected = true;
          await route.fulfill({
            status: 200,
            contentType: "application/json",
            body: JSON.stringify({ success: true }),
          });
          return;
        }
        if (p.includes("ping") || p.includes("session")) {
          if (!connected) {
            await route.fulfill({
              status: 412,
              body: JSON.stringify({ error: "PRECONDITION_FAILED" }),
            });
            return;
          }
          await route.fulfill({
            status: 200,
            contentType: "application/json",
            body: JSON.stringify({
              status: "ok",
              timestamp: new Date().toISOString(),
            }),
          });
          return;
        }

        // Thread operations
        if (p.includes("threads.list")) {
          const res = await fetch(`${rebornMock.baseUrl}/api/webchat/v2/threads`, {
            headers: { Authorization: `Bearer ${rebornMock.token}` },
          });
          await route.fulfill({
            status: res.status,
            contentType: "application/json",
            body: JSON.stringify(await res.json()),
          });
          return;
        }
        if (p.includes("threads.create")) {
          const res = await fetch(`${rebornMock.baseUrl}/api/webchat/v2/threads`, {
            method: "POST",
            headers: {
              Authorization: `Bearer ${rebornMock.token}`,
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ client_action_id: `ui-${crypto.randomUUID()}` }),
          });
          await route.fulfill({
            status: res.status,
            contentType: "application/json",
            body: JSON.stringify(await res.json()),
          });
          return;
        }
        if (p.includes("threads.getTimeline")) {
          const tid = body?.input?.id ?? "test";
          const res = await fetch(`${rebornMock.baseUrl}/api/webchat/v2/threads/${tid}/timeline`, {
            headers: { Authorization: `Bearer ${rebornMock.token}` },
          });
          await route.fulfill({
            status: res.status,
            contentType: "application/json",
            body: JSON.stringify(await res.json()),
          });
          return;
        }
        if (p.includes("threads.sendMessage")) {
          const tid = body?.input?.id ?? "test";
          const content = body?.input?.content ?? "";
          rebornMock.setScenario("stream-final-reply");
          const res = await fetch(`${rebornMock.baseUrl}/api/webchat/v2/threads/${tid}/messages`, {
            method: "POST",
            headers: {
              Authorization: `Bearer ${rebornMock.token}`,
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ content, client_action_id: `act-${crypto.randomUUID()}` }),
          });
          await route.fulfill({
            status: res.status,
            contentType: "application/json",
            body: JSON.stringify(await res.json()),
          });
          return;
        }
        if (p.includes("threads.streamEvents") || p.includes("streamEvents")) {
          const tid = body?.input?.id ?? "test";
          const sseUrl = `${rebornMock.baseUrl}/api/webchat/v2/threads/${tid}/events?token=${rebornMock.token}`;
          const res = await fetch(sseUrl);
          await route.fulfill({
            status: 200,
            contentType: "text/event-stream",
            body: await res.text(),
          });
          return;
        }

        await route.continue();
      } catch {
        await route.continue();
      }
    });
  });

  test.afterAll(async () => {
    await app?.stop();
    await rebornMock?.stop();
  });

  test("createThread opens the created thread", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    savedSettings = { tunnelUrl: rebornMock.baseUrl, apiToken: rebornMock.token };
    connected = true;

    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(3000);

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 15000 });
    await newThreadBtn.click();

    // Thread should be opened — composer should appear
    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });
  });

  test("sendMessage uses accepted response to seed run state", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    savedSettings = { tunnelUrl: rebornMock.baseUrl, apiToken: rebornMock.token };
    connected = true;

    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(3000);

    // Create and open thread
    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 15000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });

    // Send message
    await chatInput.fill("hello regression");
    await page.getByRole("button", { name: /send/i }).click();

    // User bubble appears
    await expect(page.getByText("hello regression")).toBeVisible({ timeout: 5000 });

    // Running state should appear from accepted response
    await expect(page.getByText(/Message received, waiting|Thinking/i)).toBeVisible({
      timeout: 15000,
    });
  });

  test("final reply renders from SSE", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    savedSettings = { tunnelUrl: rebornMock.baseUrl, apiToken: rebornMock.token };
    connected = true;

    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(3000);

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 15000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });

    await chatInput.fill("test final reply");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText("Here is my final answer")).toBeVisible({ timeout: 30000 });
  });
});
