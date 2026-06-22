import { expect, test } from "@playwright/test";
import { loginAnonymously, setupIronclawApiMock } from "../helpers/playwright-auth";
import { type RebornAppHost, startRebornApp } from "../helpers/reborn-app";

test.describe("Settings - IronClaw", () => {
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

  test("open /settings/ironclaw and save tunnel URL + token", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);

    await page.goto(`${app.baseUrl}/settings/ironclaw`, { waitUntil: "networkidle" });

    await page.getByLabel("Tunnel URL").fill(app.rebornBaseUrl);
    await page.getByLabel("API Token").fill(app.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();

    await expect(page.getByText("IronClaw settings saved")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("page reload preserves values", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);

    await page.goto(`${app.baseUrl}/settings/ironclaw`, { waitUntil: "networkidle" });

    await page.getByLabel("Tunnel URL").fill(app.rebornBaseUrl);
    await page.getByLabel("API Token").fill(app.rebornToken);
    await page.getByRole("button", { name: /save settings/i }).click();

    await expect(page.getByText("IronClaw settings saved")).toBeVisible();

    await page.reload();
    await page.waitForLoadState("networkidle");

    const urlValue = await page.getByLabel("Tunnel URL").inputValue();
    const tokenValue = await page.getByLabel("API Token").inputValue();
    expect(urlValue).toBe(app.rebornBaseUrl);
    expect(tokenValue).toBe(app.rebornToken);
    expect(pageErrors).toHaveLength(0);
  });
});
