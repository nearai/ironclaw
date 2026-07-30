import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("Callout", () => {
  test("info callout renders as a status live region (dark)", async ({ page }) => {
    await gotoStory(page, "components-callout--info");
    const callout = page.getByRole("status");
    await expect(callout).toBeVisible();
    await expect(callout).toContainText("Workspace refreshed.");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("info callout renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-callout--info", { theme: "light" });
    await expect(page.getByRole("status")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("tones story renders info/success/warning/danger with alert semantics", async ({ page }) => {
    await gotoStory(page, "components-callout--tones");
    await expect(page.getByText("Something informational happened.")).toBeVisible();
    await expect(page.getByText("Saved successfully.")).toBeVisible();
    await expect(
      page.getByText("Scheduler is off — automations will not run.")
    ).toBeVisible();
    const alert = page.getByRole("alert");
    await expect(alert).toContainText("Failed to reach the gateway.");
    const infoCallout = page.getByRole("status").first();
    const infoBackground = await computedStyle(infoCallout, "background-color");
    await expect(alert).not.toHaveCSS("background-color", infoBackground);
  });

  test("titled callout renders the title, body and action slot", async ({ page }) => {
    await gotoStory(page, "components-callout--titled-with-actions");
    const callout = page.getByRole("status");
    await expect(callout).toContainText("Restart required");
    await expect(callout).toContainText(
      "Networking changes take effect after the gateway restarts."
    );
    await expect(page.getByRole("button", { name: "Restart now" })).toBeVisible();
  });

  test("dismiss button brightens on hover", async ({ page }) => {
    await gotoStory(page, "components-callout--dismissible");
    const dismiss = page.getByRole("button", { name: "Dismiss" });
    await expect(dismiss).toBeVisible();
    await expect(dismiss).toHaveCSS("opacity", "0.7");
    await dismiss.hover();
    await expect(dismiss).toHaveCSS("opacity", "1");
  });

  test("dismiss button is keyboard reachable with a focus ring", async ({ page }) => {
    await gotoStory(page, "components-callout--dismissible");
    const dismiss = page.getByRole("button", { name: "Dismiss" });
    await page.keyboard.press("Tab");
    await expect(dismiss).toBeFocused();
    await expect(dismiss).not.toHaveCSS("box-shadow", "none");
  });
});
