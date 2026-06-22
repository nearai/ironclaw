import { expect, test } from "@playwright/test";
import { loginAnonymously } from "../helpers/playwright-auth";
import { type RebornAppHost, startRebornApp } from "../helpers/reborn-app";

test.describe("Navigation shell", () => {
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

  test("login works", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await expect(page).toHaveURL(/\/home/);
    expect(pageErrors).toHaveLength(0);
  });

  test("/home loads", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await expect(page.getByText("Workspace")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("/settings loads", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/settings`, { waitUntil: "networkidle" });
    await expect(page.getByText("Settings")).toBeVisible();
    await expect(page.getByRole("tab", { name: "Profile" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "IronClaw" })).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("/settings/ironclaw loads", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/settings/ironclaw`, { waitUntil: "networkidle" });
    await expect(page.getByText("IronClaw Connection")).toBeVisible();
    await expect(page.getByLabel("Tunnel URL")).toBeVisible();
    await expect(page.getByLabel("API Token")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("/setup guide loads", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    await page.goto(`${app.baseUrl}/setup`, { waitUntil: "networkidle" });
    await expect(page.getByText("IronClaw Setup")).toBeVisible();
    await expect(page.getByText("Get Your NEAR AI API Key")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });
});
