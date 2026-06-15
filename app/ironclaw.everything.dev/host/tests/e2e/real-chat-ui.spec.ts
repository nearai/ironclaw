import { expect, test } from "@playwright/test";
import { type RealStackHandle, startRealStack } from "../helpers/real-stack";

test.describe("Real chat UI", () => {
  test.describe.configure({ mode: "serial" });

  let stack: RealStackHandle;
  let pageErrors: string[];
  let consoleWarnings: string[];

  test.beforeAll(async () => {
    stack = await startRealStack();
  });

  test.beforeEach(async ({ page }) => {
    pageErrors = [];
    consoleWarnings = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("console", (msg) => {
      if (msg.type() === "warning") consoleWarnings.push(msg.text());
    });
  });

  test.afterAll(async () => {
    await stack?.stop();
  });

  test("full chat: login, save settings, create thread, send message, see reply", async ({
    page,
  }) => {
    test.setTimeout(240000);
    test.slow();

    // Mock only settings/ping at browser level (DB ops hang in dev stack)
    // Thread operations flow through the real app/plugin stack
    let savedSettings = { tunnelUrl: "", apiToken: "" };
    let connected = false;

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
          savedSettings = { tunnelUrl: stack.rebornBaseUrl, apiToken: stack.rebornToken };
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
              tenant_id: "test",
              user_id: "test",
              capabilities: { operatorWebuiConfig: true },
            }),
          });
          return;
        }

        await route.continue();
      } catch {
        await route.continue();
      }
    });

    // 1. Real anonymous login
    await page.goto(`${stack.appBaseUrl}/login`, { waitUntil: "domcontentloaded" });
    await expect(page.getByRole("button", { name: /anonymous/i })).toBeVisible({ timeout: 30000 });
    await page.getByRole("button", { name: /anonymous/i }).click();
    await page.waitForURL(/\/home/, { timeout: 30000 });

    // 2. Save settings
    await page.goto(`${stack.appBaseUrl}/settings/ironclaw`, { waitUntil: "domcontentloaded" });
    await expect(page.getByLabel("Tunnel URL")).toBeVisible({ timeout: 15000 });
    await page.getByLabel("Tunnel URL").fill(stack.rebornBaseUrl);
    await page.getByLabel("API Token").fill(stack.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();
    await expect(page.getByText("IronClaw settings saved")).toBeVisible({ timeout: 30000 });

    // 3. Navigate to chat page
    await page.goto(`${stack.appBaseUrl}/`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(3000);

    // 4. Verify no Application error
    const appError = page.getByText("Application error");
    await expect(appError).not.toBeVisible({ timeout: 3000 });

    // 5. Create thread
    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 30000 });
    await newThreadBtn.click();

    // 6. Verify no "Failed to create thread"
    const failedCreate = page.getByText("Failed to create thread");
    await expect(failedCreate).not.toBeVisible({ timeout: 5000 });

    // 7. Verify thread is opened and composer appears
    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });

    // 8. Send message
    await chatInput.fill("hello");
    await page.getByRole("button", { name: /send/i }).click();

    // 9. User bubble appears
    await expect(page.getByText("hello")).toBeVisible({ timeout: 5000 });

    // 10. Running state appears
    const runningState = page.getByText(/Message received, waiting|Thinking/i);
    await expect(
      runningState.or(page.getByText(/Here is my final answer|How can I help|Hello! I'm/)),
    ).toBeVisible({ timeout: 30000 });

    // 11. Final reply appears
    const finalReply = page.getByText(/How can I help|Hello! I'm|Here is my final|answer/i);
    await expect(finalReply).toBeVisible({ timeout: 90000 });

    // 12. No stuck reconnect banner on happy path
    const reconnect = page.getByText("Reconnecting to event stream");
    await expect(reconnect).not.toBeVisible({ timeout: 3000 });

    // 13. No page errors
    expect(pageErrors).toHaveLength(0);
  });
});
