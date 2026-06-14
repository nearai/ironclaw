import { expect, test } from "@playwright/test";
import { type RebornAppHost, startRebornApp } from "../helpers/reborn-app";
import { loginAnonymously, setupIronclawApiMock } from "../helpers/playwright-auth";

test.describe("Chat failure modes", () => {
  test.describe.configure({ mode: "serial" });

  let app: RebornAppHost;
  let pageErrors: string[];

  test.beforeAll(async () => {
    app = await startRebornApp();
  });

  test.beforeEach(async ({ page }) => {
    pageErrors = [];
    page.on("pageerror", (error) => {
      pageErrors.push(error.message);
    });
  });

  test.afterAll(async () => {
    await app?.stop();
  });

  test("unreachable backend shows disconnected state", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    await expect(page.getByText("IronClaw not set up")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("gate event shows gate banner", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);
    app.setRebornScenario("stream-gate");

    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 10000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 5000 });
    await chatInput.fill("Execute a tool");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText(/gate requires resolution/i)).toBeVisible({ timeout: 8000 });
    expect(pageErrors).toHaveLength(0);
  });

  test("failed event shows failed banner", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);
    app.setRebornScenario("stream-failed");

    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 10000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 5000 });
    await chatInput.fill("Do something");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText(/run failed/i)).toBeVisible({ timeout: 8000 });
    expect(pageErrors).toHaveLength(0);
  });

  test("cancelled event shows cancelled banner", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);
    app.setRebornScenario("stream-cancelled");

    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 10000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 5000 });
    await chatInput.fill("Stop this");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText(/run cancelled/i)).toBeVisible({ timeout: 8000 });
    expect(pageErrors).toHaveLength(0);
  });
});
