import { expect, test } from "@playwright/test";
import { loginAnonymously } from "../helpers/playwright-auth";
import { type RebornAppHost, startRebornApp } from "../helpers/reborn-app";

test.describe("App API proof — real path through plugin stack", () => {
  test.describe.configure({ mode: "serial" });

  let app: RebornAppHost;
  let pageErrors: string[];
  let consoleWarnings: string[];

  test.beforeAll(async () => {
    app = await startRebornApp();
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
    await app?.stop();
  });

  test("full chat flow through real app API", async ({ page }) => {
    test.setTimeout(60000);

    // Intercept ONLY auth routes — let ALL ironclaw RPC flow through the app API
    await loginAnonymously(page, app.baseUrl);
    app.setRebornScenario("stream-final-reply");

    // Save settings through the real settings page (stores credentials in app API DB)
    await page.goto(`${app.baseUrl}/settings/ironclaw`, { waitUntil: "domcontentloaded" });
    await expect(page.getByLabel("Tunnel URL")).toBeVisible({ timeout: 15000 });
    await page.getByLabel("Tunnel URL").fill(app.rebornBaseUrl);
    await page.getByLabel("API Token").fill(app.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();
    await expect(page.getByText("IronClaw settings saved")).toBeVisible({ timeout: 30000 });

    // Navigate to chat page — thread operations go through app API → ironclaw plugin → mock reborn
    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });

    // Wait for threads to load (may be slow on first call as plugin resolves credentials)
    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 30000 });

    // Create a thread
    await newThreadBtn.click();

    // Verify the thread is opened and composer appears
    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });

    // Send a message
    await chatInput.fill("Hello through the real API");
    await page.getByRole("button", { name: /send/i }).click();

    // User bubble appears
    await expect(page.getByText("Hello through the real API")).toBeVisible({ timeout: 5000 });

    // Running state appears from accepted response
    await expect(
      page.getByText(/Message received, waiting|Accepted, waiting|Thinking/i),
    ).toBeVisible({ timeout: 15000 });

    // Final reply renders from SSE
    await expect(page.getByText("Here is my final answer")).toBeVisible({ timeout: 30000 });

    // No page errors
    expect(pageErrors).toHaveLength(0);
  });

  test("settings not configured → thread list shows empty state", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);

    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });

    // Thread list should show empty state (no credentials yet)
    await expect(page.getByText("No threads yet")).toBeVisible({ timeout: 15000 });

    expect(pageErrors).toHaveLength(0);
  });

  test("gate event renders gate banner through app API", async ({ page }) => {
    test.setTimeout(60000);

    await loginAnonymously(page, app.baseUrl);
    app.setRebornScenario("stream-gate");

    // Save settings
    await page.goto(`${app.baseUrl}/settings/ironclaw`, { waitUntil: "domcontentloaded" });
    await expect(page.getByLabel("Tunnel URL")).toBeVisible({ timeout: 15000 });
    await page.getByLabel("Tunnel URL").fill(app.rebornBaseUrl);
    await page.getByLabel("API Token").fill(app.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();
    await expect(page.getByText("IronClaw settings saved")).toBeVisible({ timeout: 30000 });

    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 30000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });
    await chatInput.fill("Trigger gate");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText(/Approve tool execution/i)).toBeVisible({ timeout: 15000 });
    expect(pageErrors).toHaveLength(0);
  });

  test("events cycle through threads without cross-contamination", async ({ page }) => {
    test.setTimeout(60000);

    await loginAnonymously(page, app.baseUrl);
    app.setRebornScenario("stream-final-reply");

    // Save settings
    await page.goto(`${app.baseUrl}/settings/ironclaw`, { waitUntil: "domcontentloaded" });
    await expect(page.getByLabel("Tunnel URL")).toBeVisible({ timeout: 15000 });
    await page.getByLabel("Tunnel URL").fill(app.rebornBaseUrl);
    await page.getByLabel("API Token").fill(app.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();
    await expect(page.getByText("IronClaw settings saved")).toBeVisible({ timeout: 30000 });

    await page.goto(`${app.baseUrl}/`, { waitUntil: "domcontentloaded" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 30000 });

    // Create and send message in first thread
    await newThreadBtn.click();
    const chatInput = page.getByPlaceholder(/Type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 10000 });
    await chatInput.fill("First message");
    await page.getByRole("button", { name: /send/i }).click();
    await expect(page.getByText("First message")).toBeVisible({ timeout: 5000 });

    // Wait for reply to appear
    await expect(page.getByText("Here is my final answer")).toBeVisible({ timeout: 30000 });

    // Create a second thread (closes first thread's stream, starts new one)
    await newThreadBtn.click();
    const chatInput2 = page.getByRole("button", { name: /send/i });
    await expect(chatInput2).toBeDisabled({ timeout: 5000 });

    // The "run cancelled" banner should NOT appear for the second thread
    // (previous thread's cancelled state is not leaking)
    await expect(page.getByText("Run cancelled")).not.toBeVisible({ timeout: 3000 });

    expect(pageErrors).toHaveLength(0);
  });
});
