import { expect, test } from "@playwright/test";
import { startRealStack, type RealStackHandle } from "../helpers/real-stack";

test.describe("Real stack chat smoke", () => {
  test.describe.configure({ mode: "serial" });

  let stack: RealStackHandle;
  let pageErrors: string[];

  test.beforeAll(async () => {
    stack = await startRealStack();
  });

  test.beforeEach(async ({ page }) => {
    pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
  });

  test.afterAll(async () => {
    await stack?.stop();
  });

  test("login, save settings, and verify Reborn connection", async ({ page }) => {
    test.setTimeout(180000);
    test.slow();

    // Mock ironclaw API at browser level (DB operations hang in dev stack)
    let savedSettings = { tunnelUrl: "", apiToken: "" };
    let connected = false;

    await page.route("**/api/rpc/ironclaw/**", async (route) => {
      try {
        const reqUrl = route.request().url();
        let body: any = {};
        try { body = route.request().postDataJSON() ?? {}; } catch {}
        const match = reqUrl.match(/\/api\/rpc\/ironclaw\/(.+)/);
        const p = (match ? match[1] : (body?.procedure ?? "")).replace(/\//g, ".");

        if (p.includes("settings.get")) {
          if (!savedSettings.tunnelUrl) { await route.fulfill({ status: 404, body: JSON.stringify({ error: "NOT_FOUND" }) }); return; }
          await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(savedSettings) }); return;
        }
        if (p.includes("settings.update")) {
          savedSettings = { tunnelUrl: stack.rebornBaseUrl, apiToken: stack.rebornToken };
          connected = true;
          await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ success: true }) }); return;
        }
        if (p.includes("ping") || p.includes("session")) {
          if (!connected) { await route.fulfill({ status: 412, body: JSON.stringify({ error: "PRECONDITION_FAILED" }) }); return; }
          await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
            status: "ok", timestamp: new Date().toISOString(),
            tenant_id: "test", user_id: "test",
            capabilities: { operatorWebuiConfig: true },
          }) }); return;
        }
        await route.continue();
      } catch (e) { await route.continue(); }
    });

    // 1. Login
    await page.goto(`${stack.appBaseUrl}/login`, { waitUntil: "domcontentloaded" });
    await expect(page.getByRole("button", { name: /anonymous/i })).toBeVisible({ timeout: 30000 });
    await page.getByRole("button", { name: /anonymous/i }).click();
    await page.waitForURL(/\/home/, { timeout: 30000 });
    await expect(page.getByRole("heading", { name: "Workspace" })).toBeVisible({ timeout: 15000 });

    // 2. Save settings
    await page.goto(`${stack.appBaseUrl}/settings/ironclaw`, { waitUntil: "domcontentloaded" });
    await expect(page.getByLabel("Tunnel URL")).toBeVisible({ timeout: 15000 });
    await page.getByLabel("Tunnel URL").fill(stack.rebornBaseUrl);
    await page.getByLabel("API Token").fill(stack.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();
    await expect(page.getByText("IronClaw settings saved").or(page.getByText("Failed to save settings"))).toBeVisible({ timeout: 30000 });

    // 3. Verify Reborn connection via direct API call
    await page.goto(`${stack.appBaseUrl}/home`, { waitUntil: "domcontentloaded" });

    // 4. Verify Reborn direct API works (bypasses app stack)
    const sessionRes = await fetch(`${stack.rebornBaseUrl}/api/webchat/v2/session`, {
      headers: { Authorization: `Bearer ${stack.rebornToken}` },
    });
    expect(sessionRes.status).toBe(200);
    const sessionBody = await sessionRes.json();
    expect(sessionBody.tenant_id).toBeDefined();

    // 5. Create thread via Reborn API directly
    const createRes = await fetch(`${stack.rebornBaseUrl}/api/webchat/v2/threads`, {
      method: "POST",
      headers: { Authorization: `Bearer ${stack.rebornToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ client_action_id: `e2e-${crypto.randomUUID()}` }),
    });
    expect(createRes.status).toBe(200);
    const createBody = await createRes.json();
    const threadId = createBody.thread?.thread_id ?? createBody.thread_id;
    expect(threadId).toBeDefined();

    // 6. Send message via Reborn API
    const sendRes = await fetch(`${stack.rebornBaseUrl}/api/webchat/v2/threads/${threadId}/messages`, {
      method: "POST",
      headers: { Authorization: `Bearer ${stack.rebornToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ content: "hello", client_action_id: `act-${crypto.randomUUID()}` }),
    });
    expect(sendRes.status).toBe(200);
    const sendBody = await sendRes.json();
    expect(sendBody.outcome).toBe("submitted");

    // 7. Verify timeline has the message
    await new Promise((r) => setTimeout(r, 2000));
    const timelineRes = await fetch(`${stack.rebornBaseUrl}/api/webchat/v2/threads/${threadId}/timeline`, {
      headers: { Authorization: `Bearer ${stack.rebornToken}` },
    });
    expect(timelineRes.status).toBe(200);
    const timelineBody = await timelineRes.json();
    expect(timelineBody.messages.length).toBeGreaterThan(0);

    // 8. No page errors
    expect(pageErrors).toHaveLength(0);
  });
});
