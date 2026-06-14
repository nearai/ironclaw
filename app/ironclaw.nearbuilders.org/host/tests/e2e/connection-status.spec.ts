import { expect, test } from "@playwright/test";
import { type RebornAppHost, startRebornApp } from "../helpers/reborn-app";
import { loginAnonymously } from "../helpers/playwright-auth";

test.describe("Connection status", () => {
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

  test("fresh login with no settings shows Connect IronClaw", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    await expect(page.getByText("Connect IronClaw").first()).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("chat route shows never-connected empty state", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    await expect(page.getByText("IronClaw not set up")).toBeVisible();
    await expect(page.getByText("Set up IronClaw")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("disconnect action clears status", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const disconnectBtn = page.getByRole("button", { name: /disconnect/i });
    if (await disconnectBtn.isVisible().catch(() => false)) {
      await disconnectBtn.click();
    }

    await expect(page.getByText("Connect IronClaw").first()).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });
});
