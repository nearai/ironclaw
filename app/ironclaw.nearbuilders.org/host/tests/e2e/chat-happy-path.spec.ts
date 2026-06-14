import { expect, test } from "@playwright/test";
import { type RebornAppHost, startRebornApp } from "../helpers/reborn-app";
import { loginAnonymously, setupIronclawApiMock } from "../helpers/playwright-auth";

test.describe("Chat happy path", () => {
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

  test("create thread after connecting", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);
    app.setRebornScenario("healthy-chat");

    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 10000 });

    await newThreadBtn.click();
    await expect(page.getByText("No threads yet")).not.toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("send message and see user bubble", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);
    app.setRebornScenario("stream-final-reply");

    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 10000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 5000 });
    await chatInput.fill("Hello, IronClaw!");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText("Hello, IronClaw!")).toBeVisible();
    expect(pageErrors).toHaveLength(0);
  });

  test("assistant reply appears after sending", async ({ page }) => {
    await loginAnonymously(page, app.baseUrl);
    setupIronclawApiMock(page, app);
    app.setRebornScenario("stream-final-reply");

    await page.goto(`${app.baseUrl}/`, { waitUntil: "networkidle" });

    const newThreadBtn = page.getByRole("button", { name: /new thread/i });
    await expect(newThreadBtn).toBeEnabled({ timeout: 10000 });
    await newThreadBtn.click();

    const chatInput = page.getByPlaceholder(/type a message/i);
    await expect(chatInput).toBeEnabled({ timeout: 5000 });
    await chatInput.fill("What can you do?");
    await page.getByRole("button", { name: /send/i }).click();

    await expect(page.getByText("Here is my final answer")).toBeVisible({ timeout: 8000 });
    expect(pageErrors).toHaveLength(0);
  });
});
